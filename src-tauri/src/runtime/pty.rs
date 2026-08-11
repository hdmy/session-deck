use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    collections::{HashMap, VecDeque},
    fmt,
    io::{Read, Write},
    str::FromStr,
    sync::{
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    thread,
};

use portable_pty::{
    native_pty_system, Child, CommandBuilder, MasterPty, PtySize as PortablePtySize,
};
use uuid::Uuid;

use super::{ResumeSpec, RuntimeError};

pub type PtySize = PortablePtySize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PtyHandle(Uuid);

impl fmt::Display for PtyHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0.hyphenated())
    }
}

impl FromStr for PtyHandle {
    type Err = RuntimeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let uuid = Uuid::parse_str(value).map_err(|_| RuntimeError::InvalidHandle)?;
        if uuid.hyphenated().to_string() != value {
            return Err(RuntimeError::InvalidHandle);
        }
        Ok(Self(uuid))
    }
}

impl PtyHandle {
    pub fn parse(value: &str) -> Result<Self, RuntimeError> {
        value.parse()
    }
}

impl PtyHandle {
    fn fresh(registry: &HashMap<PtyHandle, Session>) -> Self {
        loop {
            let handle = Self(Uuid::new_v4());
            if !registry.contains_key(&handle) {
                return handle;
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyEvent {
    Output(Vec<u8>),
    Exited { status: String },
    Error { message: String },
}

struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    events: Receiver<PtyEvent>,
    child: Arc<Mutex<Box<dyn Child + Send>>>,
    exit_status: Arc<Mutex<Option<String>>>,
    pending_output: VecDeque<u8>,
    pending_events: VecDeque<PtyEvent>,
    reader_done: Arc<AtomicBool>,
    waiter_done: Arc<AtomicBool>,
    reader_thread: Option<thread::JoinHandle<()>>,
    waiter_thread: Option<thread::JoinHandle<()>>,
}

/// Thread-safe registry for active provider PTYs.
#[derive(Default)]
pub struct PtyManager {
    sessions: Mutex<HashMap<PtyHandle, Session>>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&self, spec: &ResumeSpec, size: PtySize) -> Result<PtyHandle, RuntimeError> {
        self.start_with_thread_failures(spec, size, false, false)
    }

    #[cfg(test)]
    fn start_with_thread_failures(
        &self,
        spec: &ResumeSpec,
        size: PtySize,
        fail_reader: bool,
        fail_waiter: bool,
    ) -> Result<PtyHandle, RuntimeError> {
        self.start_inner(spec, size, fail_reader, fail_waiter)
    }

    #[cfg(not(test))]
    fn start_with_thread_failures(
        &self,
        spec: &ResumeSpec,
        size: PtySize,
        fail_reader: bool,
        fail_waiter: bool,
    ) -> Result<PtyHandle, RuntimeError> {
        self.start_inner(spec, size, fail_reader, fail_waiter)
    }

    fn start_inner(
        &self,
        spec: &ResumeSpec,
        size: PtySize,
        fail_reader: bool,
        fail_waiter: bool,
    ) -> Result<PtyHandle, RuntimeError> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size)
            .map_err(|error| RuntimeError::Pty(error.to_string()))?;

        let mut command = CommandBuilder::new(&spec.executable);
        command.args(&spec.args);
        command.cwd(&spec.cwd);
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| RuntimeError::Pty(error.to_string()))?;
        let child: Arc<Mutex<Box<dyn Child + Send>>> = Arc::new(Mutex::new(child));
        let reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(error) => {
                terminate_child_best_effort(&child);
                return Err(RuntimeError::Pty(error.to_string()));
            }
        };
        let writer = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(error) => {
                terminate_child_best_effort(&child);
                return Err(RuntimeError::Pty(error.to_string()));
            }
        };
        // Bound queued output so an idle webview cannot cause unbounded
        // memory growth. Backpressure is preferable to silently dropping
        // transcript bytes.
        let (sender, receiver) = mpsc::sync_channel(256);
        let exit_status = Arc::new(Mutex::new(None));
        let reader_done = Arc::new(AtomicBool::new(false));
        let waiter_done = Arc::new(AtomicBool::new(false));

        // Acquire the registry before starting background threads. If the
        // registry is poisoned, clean up the already-spawned child instead of
        // leaving an untracked process behind.
        let mut sessions = match self.sessions.lock() {
            Ok(sessions) => sessions,
            Err(_) => {
                terminate_child_best_effort(&child);
                return Err(RuntimeError::Pty("PTY registry lock poisoned".to_owned()));
            }
        };
        let handle = PtyHandle::fresh(&sessions);
        let reader_thread = match if fail_reader {
            Err(std::io::Error::other("injected reader thread failure"))
        } else {
            spawn_reader(reader, sender.clone(), reader_done.clone())
        } {
            Ok(thread) => thread,
            Err(error) => {
                terminate_child_best_effort(&child);
                return Err(RuntimeError::Pty(format!("reader thread: {error}")));
            }
        };
        let waiter_thread = match if fail_waiter {
            Err(std::io::Error::other("injected waiter thread failure"))
        } else {
            spawn_waiter(
                child.clone(),
                exit_status.clone(),
                sender,
                waiter_done.clone(),
            )
        } {
            Ok(thread) => thread,
            Err(error) => {
                terminate_child_best_effort(&child);
                join_if_finished(Some(reader_thread));
                return Err(RuntimeError::Pty(format!("waiter thread: {error}")));
            }
        };

        sessions.insert(
            handle,
            Session {
                master: pair.master,
                writer,
                events: receiver,
                child,
                exit_status,
                pending_output: VecDeque::new(),
                pending_events: VecDeque::new(),
                reader_done,
                waiter_done,
                reader_thread: Some(reader_thread),
                waiter_thread: Some(waiter_thread),
            },
        );
        Ok(handle)
    }

    pub fn write(&self, handle: PtyHandle, bytes: &[u8]) -> Result<(), RuntimeError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| RuntimeError::Pty("PTY registry lock poisoned".to_owned()))?;
        let session = sessions
            .get_mut(&handle)
            .ok_or(RuntimeError::UnknownHandle)?;
        session.writer.write_all(bytes)?;
        session.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, handle: PtyHandle, size: PtySize) -> Result<(), RuntimeError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| RuntimeError::Pty("PTY registry lock poisoned".to_owned()))?;
        let session = sessions
            .get_mut(&handle)
            .ok_or(RuntimeError::UnknownHandle)?;
        session
            .master
            .resize(size)
            .map_err(|error| RuntimeError::Pty(error.to_string()))?;
        Ok(())
    }

    pub fn read_events(&self, handle: PtyHandle) -> Result<Vec<PtyEvent>, RuntimeError> {
        self.read_events_limited(handle, usize::MAX, usize::MAX)
    }

    pub fn read_events_limited(
        &self,
        handle: PtyHandle,
        max_events: usize,
        max_bytes: usize,
    ) -> Result<Vec<PtyEvent>, RuntimeError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| RuntimeError::Pty("PTY registry lock poisoned".to_owned()))?;
        let session = sessions
            .get_mut(&handle)
            .ok_or(RuntimeError::UnknownHandle)?;
        if max_events == 0 || max_bytes == 0 {
            return Ok(Vec::new());
        }
        let mut bytes = 0;
        let mut events = Vec::new();
        while events.len() < max_events {
            let remaining = max_bytes.saturating_sub(bytes);
            if !session.pending_output.is_empty() {
                let take = remaining.min(session.pending_output.len());
                if take == 0 {
                    break;
                }
                let output = session.pending_output.drain(..take).collect::<Vec<_>>();
                bytes += output.len();
                events.push(PtyEvent::Output(output));
                continue;
            }
            let event = if let Some(event) = session.pending_events.pop_front() {
                event
            } else {
                let Ok(event) = session.events.try_recv() else {
                    break;
                };
                event
            };
            match event {
                PtyEvent::Output(output) => {
                    let take = remaining.min(output.len());
                    if take == 0 {
                        session.pending_output.extend(output);
                        break;
                    }
                    let (head, tail) = output.split_at(take);
                    events.push(PtyEvent::Output(head.to_vec()));
                    bytes += take;
                    session.pending_output.extend(tail.iter().copied());
                }
                other => events.push(other),
            }
            if bytes >= max_bytes {
                break;
            }
        }
        Ok(events)
    }

    /// Completion barrier for continuation output. Producers must be done and
    /// all queued events/output bytes must have been delivered.
    pub fn events_drained(&self, handle: PtyHandle) -> Result<bool, RuntimeError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| RuntimeError::Pty("PTY registry lock poisoned".to_owned()))?;
        let session = sessions
            .get_mut(&handle)
            .ok_or(RuntimeError::UnknownHandle)?;
        if !session.reader_done.load(Ordering::Acquire)
            || !session.waiter_done.load(Ordering::Acquire)
        {
            return Ok(false);
        }
        if let Ok(event) = session.events.try_recv() {
            session.pending_events.push_back(event);
            return Ok(false);
        }
        Ok(session.pending_output.is_empty() && session.pending_events.is_empty())
    }

    pub fn exit_status(&self, handle: PtyHandle) -> Result<Option<String>, RuntimeError> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| RuntimeError::Pty("PTY registry lock poisoned".to_owned()))?;
        let session = sessions.get(&handle).ok_or(RuntimeError::UnknownHandle)?;
        let status = session
            .exit_status
            .lock()
            .map_err(|_| RuntimeError::Pty("PTY session lock poisoned".to_owned()))?
            .clone();
        Ok(status)
    }

    /// Returns the observed process exit status, if the child has exited.
    pub fn exit(&self, handle: PtyHandle) -> Result<Option<String>, RuntimeError> {
        self.exit_status(handle)
    }

    /// Returns handles whose direct child has not reported exit.
    ///
    /// This is intentionally not an output-drain or lifecycle-gate check:
    /// direct children may have exited while reader/waiter threads still have
    /// queued UI output. Use `events_drained` and `reap_if_inactive` when the
    /// completion barrier is required.
    pub fn active_handles(&self) -> Result<Vec<PtyHandle>, RuntimeError> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| RuntimeError::Pty("PTY registry lock poisoned".to_owned()))?;
        let mut handles = sessions
            .iter()
            .filter_map(|(handle, session)| match session.exit_status.lock() {
                Ok(status) => status.is_none().then_some(*handle),
                // A poisoned status slot is not evidence of process exit. Keep
                // the lifecycle gate active until close can kill and wait.
                Err(_) => Some(*handle),
            })
            .collect::<Vec<_>>();
        handles.sort_by_key(|handle| handle.0);
        Ok(handles)
    }

    /// Removes an exited session from the registry. Missing handles are also
    /// considered inactive so a stale application gate can be cleared.
    pub fn reap_if_inactive(&self, handle: PtyHandle) -> Result<bool, RuntimeError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| RuntimeError::Pty("PTY registry lock poisoned".to_owned()))?;
        let Some(session) = sessions.get_mut(&handle) else {
            return Ok(true);
        };
        let mut inactive = match session.exit_status.lock() {
            Ok(status) => status.is_some(),
            // A poisoned status slot is not an exit observation. Keep the
            // session registered so the lifecycle gate cannot be cleared.
            Err(_) => false,
        };
        if inactive {
            inactive = session.reader_done.load(Ordering::Acquire)
                && session.waiter_done.load(Ordering::Acquire)
                && session.pending_output.is_empty()
                && session.pending_events.is_empty();
            if inactive {
                if let Ok(event) = session.events.try_recv() {
                    session.pending_events.push_back(event);
                    inactive = false;
                }
            }
        }
        let session = inactive.then(|| sessions.remove(&handle)).flatten();
        drop(sessions);
        if let Some(session) = session {
            finish_session(session);
        }
        Ok(inactive)
    }

    pub fn close(&self, handle: PtyHandle) -> Result<(), RuntimeError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| RuntimeError::Pty("PTY registry lock poisoned".to_owned()))?;
        let session = sessions
            .get_mut(&handle)
            .ok_or(RuntimeError::UnknownHandle)?;
        if session_status_observed(session) {
            let session = sessions.remove(&handle).expect("session was just checked");
            drop(sessions);
            finish_session(session);
            return Ok(());
        }

        // Keep the session in the registry and hold the lifecycle gate until
        // the child has actually been reaped. `kill` only requests exit and
        // returning immediately would let a scan race the still-running CLI.
        // Explicit close is user cancellation; any output not yet drained by
        // the UI is intentionally discarded when the session is removed.
        // Do not hold the exit-status mutex while taking the child mutex: the
        // waiter takes those locks in the opposite order.
        let mut child = match session.child.lock() {
            Ok(child) => child,
            Err(poisoned) => poisoned.into_inner(),
        };
        if session_status_observed(session) {
            drop(child);
            let session = sessions.remove(&handle).expect("session still registered");
            drop(sessions);
            finish_session(session);
            return Ok(());
        }
        match child.kill() {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(RuntimeError::Pty(error.to_string())),
        }
        let wait_result = child.wait();
        drop(child);
        let status = match wait_result {
            Ok(status) => format!("{status:?}"),
            Err(error) => return Err(RuntimeError::Pty(error.to_string())),
        };
        match session.exit_status.lock() {
            Ok(mut slot) => *slot = Some(status),
            Err(poisoned) => *poisoned.into_inner() = Some(status),
        }
        let session = sessions.remove(&handle).expect("session still registered");
        drop(sessions);
        finish_session(session);
        Ok(())
    }

    pub fn shutdown_all(&self) -> Result<(), RuntimeError> {
        let handles = self
            .sessions
            .lock()
            .map_err(|_| RuntimeError::Pty("PTY registry lock poisoned".to_owned()))?
            .keys()
            .copied()
            .collect::<Vec<_>>();
        let mut first_error = None;
        for handle in handles {
            if let Err(error) = self.close(handle) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl Drop for PtyManager {
    fn drop(&mut self) {
        let sessions = match self.sessions.get_mut() {
            Ok(sessions) => std::mem::take(sessions),
            Err(poisoned) => std::mem::take(poisoned.into_inner()),
        };
        for (_, session) in sessions {
            terminate_child_best_effort(&session.child);
            finish_session(session);
        }
    }
}

fn terminate_child_best_effort(child: &Arc<Mutex<Box<dyn Child + Send>>>) {
    let mut child = match child.lock() {
        Ok(child) => child,
        Err(poisoned) => poisoned.into_inner(),
    };
    let _ = child.kill();
    let _ = child.wait();
}

fn session_status_observed(session: &Session) -> bool {
    session
        .exit_status
        .lock()
        .map(|status| status.is_some())
        .unwrap_or(false)
}

fn spawn_reader(
    mut reader: Box<dyn Read + Send>,
    sender: SyncSender<PtyEvent>,
    done: Arc<AtomicBool>,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("context-vault-pty-reader".to_owned())
        .spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if sender
                            .send(PtyEvent::Output(buffer[..count].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            done.store(true, Ordering::Release);
        })
}

fn spawn_waiter(
    child: Arc<Mutex<Box<dyn Child + Send>>>,
    exit_status: Arc<Mutex<Option<String>>>,
    sender: SyncSender<PtyEvent>,
    done: Arc<AtomicBool>,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("context-vault-pty-waiter".to_owned())
        .spawn(move || {
            loop {
                let result = match child.lock() {
                    Ok(mut child) => child.try_wait().map_err(|error| error.to_string()),
                    Err(_) => {
                        let _ = sender.try_send(PtyEvent::Error {
                            message: "PTY child lock poisoned".to_owned(),
                        });
                        // A poisoned child lock is not an exit observation. Retain the
                        // gate and let close recover the guard, kill, and wait.
                        break;
                    }
                };
                match result {
                    Ok(Some(status)) => {
                        let status = format!("{status:?}");
                        let mut slot = match exit_status.lock() {
                            Ok(slot) => slot,
                            Err(_) => {
                                let _ = sender.try_send(PtyEvent::Error {
                                    message: "PTY exit-status lock poisoned".to_owned(),
                                });
                                // Do not turn an undeliverable status into an inactive
                                // session. close will perform the authoritative wait.
                                break;
                            }
                        };
                        if slot.is_some() {
                            break;
                        }
                        *slot = Some(status.clone());
                        let _ = sender.send(PtyEvent::Exited { status });
                        break;
                    }
                    Ok(None) => thread::sleep(std::time::Duration::from_millis(20)),
                    Err(message) => {
                        let _ = sender.try_send(PtyEvent::Error { message });
                        // try_wait errors do not prove that the direct child exited.
                        // Keep the gate active; close can kill and wait it safely.
                        break;
                    }
                }
            }
            done.store(true, Ordering::Release);
        })
}

/// Drop PTY resources before bounded thread cleanup. A reader can remain
/// blocked if a descendant still owns the slave; in that case its JoinHandle is
/// detached rather than making close/drop block indefinitely.
fn finish_session(session: Session) {
    let Session {
        master,
        writer,
        events,
        child,
        exit_status,
        pending_output,
        pending_events,
        reader_done,
        waiter_done,
        reader_thread,
        waiter_thread,
    } = session;
    drop(master);
    drop(writer);
    drop(events);
    drop(child);
    drop(exit_status);
    drop(pending_output);
    drop(pending_events);
    drop(reader_done);
    drop(waiter_done);
    join_if_finished(reader_thread);
    join_if_finished(waiter_thread);
}

fn join_if_finished(thread: Option<thread::JoinHandle<()>>) {
    if let Some(thread) = thread {
        if thread.is_finished() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ResumeSpec;
    use std::{ffi::OsString, path::PathBuf, time::Duration};
    use tempfile::tempdir;

    #[cfg(unix)]
    #[test]
    fn registry_tracks_and_closes_a_synthetic_process() {
        let cwd = tempdir().unwrap();
        let spec = ResumeSpec::new(
            PathBuf::from("/bin/sh"),
            [OsString::from("-c"), OsString::from("printf runtime-test")],
            cwd.path(),
        )
        .unwrap();
        let manager = PtyManager::new();
        let handle = manager
            .start(
                &spec,
                PtySize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                },
            )
            .unwrap();
        assert_eq!(manager.active_handles().unwrap(), vec![handle]);

        for _ in 0..20 {
            if manager.exit_status(handle).unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let events = manager.read_events(handle).unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, PtyEvent::Exited { .. })));
        assert!(manager.active_handles().unwrap().is_empty());
        manager.close(handle).unwrap();
        assert!(matches!(
            manager.read_events(handle),
            Err(RuntimeError::UnknownHandle)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn close_reaps_a_long_running_child_before_releasing_the_session() {
        let cwd = tempdir().unwrap();
        let spec = ResumeSpec::new(
            PathBuf::from("/bin/sh"),
            [OsString::from("-c"), OsString::from("sleep 10")],
            cwd.path(),
        )
        .unwrap();
        let manager = PtyManager::new();
        let handle = manager
            .start(
                &spec,
                PtySize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                },
            )
            .unwrap();
        let started = std::time::Instant::now();
        manager.close(handle).unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(manager.active_handles().unwrap().is_empty());
        assert!(matches!(
            manager.exit_status(handle),
            Err(RuntimeError::UnknownHandle)
        ));
        // A second reconciliation observes the handle as inactive rather than
        // racing a still-running waiter or child process.
        assert!(manager.reap_if_inactive(handle).unwrap());
    }

    #[test]
    fn handle_parse_requires_canonical_uuid() {
        let handle = PtyHandle::fresh(&HashMap::new());
        let text = handle.to_string();
        assert_eq!(PtyHandle::parse(&text).unwrap(), handle);
        assert!(PtyHandle::parse(&text.to_uppercase()).is_err());
        assert!(PtyHandle::parse("not-a-handle").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn limited_reads_preserve_output_remainder_across_polls() {
        let cwd = tempdir().unwrap();
        let expected = "0123456789abcdefghijklmnopqrstuvwxyz";
        let spec = ResumeSpec::new(
            PathBuf::from("/bin/sh"),
            [
                OsString::from("-c"),
                OsString::from(format!("printf '%s' {expected}")),
            ],
            cwd.path(),
        )
        .unwrap();
        let manager = PtyManager::new();
        let handle = manager
            .start(
                &spec,
                PtySize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                },
            )
            .unwrap();

        let mut output: Vec<u8> = Vec::new();
        for _ in 0..100 {
            for event in manager.read_events_limited(handle, 1, 3).unwrap() {
                if let PtyEvent::Output(bytes) = event {
                    output.extend(bytes);
                }
            }
            if output.len() >= expected.len() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(String::from_utf8(output).unwrap(), expected);
        manager.close(handle).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn exit_waits_for_large_output_queue_to_drain() {
        let cwd = tempdir().unwrap();
        let spec = ResumeSpec::new(
            PathBuf::from("/bin/sh"),
            [
                OsString::from("-c"),
                OsString::from("head -c 70000 /dev/zero | tr '\\0' x"),
            ],
            cwd.path(),
        )
        .unwrap();
        let manager = PtyManager::new();
        let handle = manager
            .start(
                &spec,
                PtySize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                },
            )
            .unwrap();
        // Observe direct-child exit before touching the event queue.  This is
        // deterministic: the completion barrier must remain false until the
        // 70 KiB output is explicitly consumed.
        for _ in 0..200 {
            if manager.exit_status(handle).unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(manager.exit_status(handle).unwrap().is_some());
        assert!(manager.active_handles().unwrap().is_empty());
        assert!(!manager.events_drained(handle).unwrap());

        let mut output: Vec<u8> = Vec::new();
        for _ in 0..200 {
            let events = manager.read_events_limited(handle, 64, 64 * 1024).unwrap();
            output.extend(
                events
                    .iter()
                    .filter_map(|event| match event {
                        PtyEvent::Output(bytes) => Some(bytes.as_slice()),
                        _ => None,
                    })
                    .flatten(),
            );
            if manager.events_drained(handle).unwrap() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(output.len(), 70_000);
        assert!(manager.events_drained(handle).unwrap());
        manager.close(handle).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn late_output_after_exit_is_delivered_before_drain_barrier() {
        let cwd = tempdir().unwrap();
        let spec = ResumeSpec::new(
            PathBuf::from("/bin/sh"),
            [
                OsString::from("-c"),
                OsString::from("printf early; (sleep 0.05; printf late) & wait"),
            ],
            cwd.path(),
        )
        .unwrap();
        let manager = PtyManager::new();
        let handle = manager
            .start(
                &spec,
                PtySize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                },
            )
            .unwrap();
        let mut output = Vec::new();
        for _ in 0..100 {
            for event in manager.read_events_limited(handle, 64, 64 * 1024).unwrap() {
                if let PtyEvent::Output(bytes) = event {
                    output.extend(bytes);
                }
            }
            if manager.events_drained(handle).unwrap() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(output, b"earlylate");
        manager.close(handle).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn exited_sessions_can_be_reaped_without_accumulating_handles() {
        let cwd = tempdir().unwrap();
        let active_spec = ResumeSpec::new(
            PathBuf::from("/bin/sh"),
            [OsString::from("-c"), OsString::from("sleep 10")],
            cwd.path(),
        )
        .unwrap();
        let manager = PtyManager::new();
        let active_handle = manager
            .start(
                &active_spec,
                PtySize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                },
            )
            .unwrap();
        assert!(!manager.reap_if_inactive(active_handle).unwrap());
        manager.close(active_handle).unwrap();

        let exited_spec = ResumeSpec::new(
            PathBuf::from("/bin/sh"),
            [OsString::from("-c"), OsString::from("exit 0")],
            cwd.path(),
        )
        .unwrap();
        let exited_handle = manager
            .start(
                &exited_spec,
                PtySize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                },
            )
            .unwrap();
        for _ in 0..50 {
            if manager.exit_status(exited_handle).unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // Reaping is gated on the same completion barrier used by polling;
        // consume the terminal event before removing the session.
        for _ in 0..20 {
            manager.read_events(exited_handle).unwrap();
            if manager.events_drained(exited_handle).unwrap() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(manager.reap_if_inactive(exited_handle).unwrap());
        assert!(matches!(
            manager.read_events(exited_handle),
            Err(RuntimeError::UnknownHandle)
        ));
        assert!(manager.reap_if_inactive(exited_handle).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn poisoned_exit_status_never_looks_inactive() {
        let cwd = tempdir().unwrap();
        let spec = ResumeSpec::new(
            PathBuf::from("/bin/sh"),
            [OsString::from("-c"), OsString::from("sleep 10")],
            cwd.path(),
        )
        .unwrap();
        let manager = PtyManager::new();
        let handle = manager
            .start(
                &spec,
                PtySize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                },
            )
            .unwrap();
        let status = manager
            .sessions
            .lock()
            .unwrap()
            .get(&handle)
            .unwrap()
            .exit_status
            .clone();
        let poisoned = thread::spawn(move || {
            let _guard = status.lock().unwrap();
            panic!("poison test");
        })
        .join();
        assert!(poisoned.is_err());

        assert_eq!(manager.active_handles().unwrap(), vec![handle]);
        assert!(!manager.reap_if_inactive(handle).unwrap());
        manager.close(handle).unwrap();
        assert!(manager.active_handles().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn terminate_helper_kills_and_waits_for_direct_child() {
        let cwd = tempdir().unwrap();
        let pair = native_pty_system()
            .openpty(PortablePtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "sleep 10"]);
        command.cwd(cwd.path());
        let child = pair.slave.spawn_command(command).unwrap();
        let child: Arc<Mutex<Box<dyn Child + Send>>> = Arc::new(Mutex::new(child));
        terminate_child_best_effort(&child);
        let status = child.lock().unwrap().try_wait().unwrap();
        assert!(status.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn injected_reader_thread_failure_reaps_child_and_leaves_registry_empty() {
        let cwd = tempdir().unwrap();
        let spec = ResumeSpec::new(
            PathBuf::from("/bin/sh"),
            [OsString::from("-c"), OsString::from("sleep 10")],
            cwd.path(),
        )
        .unwrap();
        let manager = PtyManager::new();
        let result = manager.start_with_thread_failures(
            &spec,
            PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            },
            true,
            false,
        );
        assert!(
            matches!(result, Err(RuntimeError::Pty(message)) if message.contains("reader thread"))
        );
        assert!(manager.active_handles().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn injected_waiter_thread_failure_reaps_child_and_reader_safely() {
        let cwd = tempdir().unwrap();
        let spec = ResumeSpec::new(
            PathBuf::from("/bin/sh"),
            [OsString::from("-c"), OsString::from("sleep 10")],
            cwd.path(),
        )
        .unwrap();
        let manager = PtyManager::new();
        let result = manager.start_with_thread_failures(
            &spec,
            PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            },
            false,
            true,
        );
        assert!(
            matches!(result, Err(RuntimeError::Pty(message)) if message.contains("waiter thread"))
        );
        assert!(manager.active_handles().unwrap().is_empty());
    }
}
