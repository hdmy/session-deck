import { onScopeDispose, shallowReadonly, shallowRef } from 'vue';
import { api } from '../../api';
import type { ContinuationEvent, ContinuationStatus, LiveTranscriptEvent, ResumePreview } from '../../types';
import { continuationFinished, encodeTerminalInput, isContinuationStale } from './continuationLogic';

export type TerminalPhase = 'idle' | 'preflighting' | 'starting' | 'running' | 'draining' | 'exited' | 'error' | 'closed';
type FinishReason = 'exited' | 'closed';
export type ContinuationMode = 'resume' | 'fork';
export type ContinuationCloseResult =
  | { status: 'closed' }
  | { status: 'error'; error: string };
export interface ContinuationStartOptions {
  mode?: ContinuationMode;
  rows?: number;
  cols?: number;
}

function message(error: unknown): string { return error instanceof Error ? error.message : typeof error === 'string' ? error : 'Continuation failed.'; }

export function useContinuationTerminal(options: { onOutput?: (data: number[]) => void; onFinished?: (reason: FinishReason, status: string) => void } = {}) {
  const phase = shallowRef<TerminalPhase>('idle');
  const error = shallowRef<string | null>(null);
  const preview = shallowRef<ResumePreview | null>(null);
  const handle = shallowRef<string | null>(null);
  // A missing handle is safe to treat as closed only after the backend has
  // confirmed inactivity (or a close operation has succeeded).
  const inactiveConfirmed = shallowRef(true);
  const sessionId = shallowRef<string | null>(null);
  const parentSessionId = shallowRef<string | null>(null);
  const mode = shallowRef<ContinuationMode>('resume');
  const status = shallowRef<string | null>(null);
  const events = shallowRef<ContinuationEvent[]>([]);
  const liveEvents = shallowRef<LiveTranscriptEvent[]>([]);
  const noNewEvents = shallowRef(false);
  const tailPartial = shallowRef(false);
  const tailDiagnostics = shallowRef(0);
  const tailError = shallowRef<string | null>(null);
  const tailCaughtUp = shallowRef(false);
  let timer: number | undefined;
  let polling = false;
  let generation = 0;
  let disposed = false;
  let finishedGeneration: number | null = null;

  function clearTimer() { if (timer !== undefined) window.clearTimeout(timer); timer = undefined; }
  function isStale(token: number) { return isContinuationStale(token, generation, disposed); }
  function emitFinished(reason: FinishReason, status: string, token: number) { if (finishedGeneration === token) return; finishedGeneration = token; options.onFinished?.(reason, status); }

  function applyTail(result: ContinuationStatus) {
    // The backend owns the actual target ID. A fork reports the child in
    // session_id and keeps the locked source in parent_session_id.
    sessionId.value = result.session_id || sessionId.value;
    parentSessionId.value = result.parent_session_id ?? (mode.value === 'fork' ? parentSessionId.value : null);
    status.value = result.status;
    // Tail metadata is a cumulative observation. A later poll may contain a
    // delta without repeating the complete diagnostic count, so never let the
    // UI regress to a smaller value. Partial is the latest parse state and may
    // legitimately clear once the provider appends a complete line.
    tailPartial.value = result.tail_partial ?? false;
    tailDiagnostics.value = Math.max(tailDiagnostics.value, result.tail_diagnostics ?? 0);
    tailError.value = result.tail_error ?? null;
    tailCaughtUp.value = result.tail_caught_up ?? false;
    if (result.status.trim().toLowerCase() === 'draining') phase.value = 'draining';
    else if (phase.value === 'starting') phase.value = 'running';
    const delta = result.live_events ?? [];
    noNewEvents.value = delta.length === 0;
    if (!delta.length) return;
    const byId = new Map(liveEvents.value.map((event) => [String(event.id), event]));
    for (const event of delta) byId.set(String(event.id), event);
    liveEvents.value = [...byId.values()];
  }

  async function bestEffortClose(value: string, token: number): Promise<boolean> {
    try {
      await api.closeContinuation(value);
      if (handle.value === value) handle.value = null;
      inactiveConfirmed.value = true;
      return true;
    } catch (cause) {
      // Preserve a late/failing handle so explicit Close can retry. Dropping
      // it would incorrectly release App-level guards while active state is
      // still unknown.
      if (!handle.value) handle.value = value;
      inactiveConfirmed.value = false;
      if (!isStale(token) || handle.value === value) error.value = error.value ?? `Unable to close terminal: ${message(cause)}`;
      return false;
    }
  }

  async function fail(cause: unknown, value: string | null, token: number) {
    clearTimer(); phase.value = 'error'; error.value = message(cause); generation += 1;
    if (value) await bestEffortClose(value, token);
  }

  async function preflight(sessionId: string) {
    const token = ++generation; phase.value = 'preflighting'; error.value = null;
    try { const result = await api.resumePreflight(sessionId); if (isStale(token)) return null; preview.value = result; phase.value = 'idle'; return result; }
    catch (cause) { if (!isStale(token)) { phase.value = 'error'; error.value = message(cause); } return null; }
  }

  async function poll(token: number) {
    const value = handle.value;
    if (!value || polling || isStale(token)) return;
    polling = true;
    try {
      const result = await api.pollContinuation(value);
      if (isStale(token) || handle.value !== value) return;
      applyTail(result);
      if (result.events.length) events.value = [...events.value, ...result.events];
      for (const event of result.events) { if (event.data?.length) options.onOutput?.(event.data); }
      const failed = result.events.find((event) => event.kind === 'error');
      if (failed) { await fail(failed.message ?? result.status, value, token); return; }
      if (continuationFinished(result)) {
        clearTimer();
        // A terminal status alone is not enough to claim the handle is gone;
        // close must succeed before releasing the UI guards.
        if (!await bestEffortClose(value, token) || isStale(token)) return;
        phase.value = 'exited'; emitFinished('exited', result.status, token); return;
      }
      timer = window.setTimeout(() => void poll(token), 120);
    } catch (cause) { if (!isStale(token)) await fail(cause, value, token); }
    finally { polling = false; }
  }

  async function start(
    targetSessionId: string,
    rowsOrMode: number | ContinuationMode | ContinuationStartOptions = 24,
    colsOrMode: number | ContinuationMode = 80,
    requestedMode: ContinuationMode | number = 'resume',
  ) {
    let rows = 24;
    let cols = 80;
    let startMode: ContinuationMode = typeof requestedMode === 'number' ? 'resume' : requestedMode;
    if (typeof requestedMode === 'number') cols = requestedMode;
    if (typeof rowsOrMode === 'number') rows = rowsOrMode;
    else if (typeof rowsOrMode === 'string') startMode = rowsOrMode;
    else {
      rows = rowsOrMode.rows ?? rows;
      cols = rowsOrMode.cols ?? cols;
      startMode = rowsOrMode.mode ?? startMode;
    }
    if (typeof colsOrMode === 'number') cols = colsOrMode;
    else startMode = colsOrMode;
    const token = ++generation; finishedGeneration = null; phase.value = 'starting'; error.value = null; status.value = 'starting';
    inactiveConfirmed.value = false;
    mode.value = startMode;
    sessionId.value = targetSessionId;
    parentSessionId.value = startMode === 'fork' ? targetSessionId : null;
    events.value = [];
    liveEvents.value = [];
    noNewEvents.value = false;
    tailPartial.value = false;
    tailDiagnostics.value = 0;
    tailError.value = null;
    tailCaughtUp.value = false;
    try {
      const result = startMode === 'fork'
        ? await api.startForkContinuation(targetSessionId, rows, cols)
        : await api.startContinuation(targetSessionId, rows, cols);
      if (isStale(token) || phase.value !== 'starting') { await bestEffortClose(result.handle, token); return null; }
      handle.value = result.handle;
      inactiveConfirmed.value = false;
      sessionId.value = result.session_id || sessionId.value;
      parentSessionId.value = result.parent_session_id ?? (startMode === 'fork' ? targetSessionId : null);
      status.value = result.status;
      tailCaughtUp.value = result.tail_caught_up ?? false;
      phase.value = 'running';
      void poll(token);
      return result;
    } catch (cause) {
      // A rejected start returned no handle, so no continuation was created.
      if (!handle.value) inactiveConfirmed.value = true;
      if (!isStale(token)) { phase.value = 'error'; error.value = message(cause); }
      return null;
    }
  }

  async function write(value: string) {
    const current = handle.value; const token = generation;
    if (!current || phase.value !== 'running') return;
    try { await api.writeContinuation(current, encodeTerminalInput(value)); if (isStale(token)) return; }
    catch (cause) { if (!isStale(token)) await fail(cause, current, token); }
  }
  async function resize(rows: number, cols: number, pixelWidth = 0, pixelHeight = 0) {
    const current = handle.value; const token = generation;
    if (!current || phase.value !== 'running') return;
    try { await api.resizeContinuation(current, rows, cols, pixelWidth, pixelHeight); }
    catch (cause) { if (!isStale(token)) await fail(cause, current, token); }
  }
  async function close(): Promise<ContinuationCloseResult> {
    const token = generation;
    generation += 1;
    clearTimer();
    const current = handle.value;
    if (!current) {
      if (inactiveConfirmed.value) {
        const alreadyFinished = phase.value === 'closed' || phase.value === 'exited';
        status.value = 'closed';
        phase.value = 'closed';
        if (!alreadyFinished) emitFinished('closed', 'closed', token);
        return { status: 'closed' };
      }
      const failure = 'Unable to close terminal: no active continuation handle.';
      phase.value = 'error';
      error.value = failure;
      return { status: 'error', error: failure };
    }
    error.value = null;
    try {
      await api.closeContinuation(current);
      if (handle.value === current) handle.value = null;
      inactiveConfirmed.value = true;
      status.value = 'closed';
      phase.value = 'closed';
      emitFinished('closed', 'closed', token);
      return { status: 'closed' };
    } catch (cause) {
      const failure = message(cause);
      // Keep the handle so a user can retry. The parent must not unlock or
      // refresh until the backend confirms that this handle is closed.
      if (handle.value === current) {
        phase.value = 'error';
        error.value = failure;
      }
      return { status: 'error', error: failure };
    }
  }

  onScopeDispose(() => {
    disposed = true; generation += 1; clearTimer();
    const current = handle.value;
    if (current) void api.closeContinuation(current).then(() => { if (handle.value === current) handle.value = null; }).catch(() => undefined);
  });

  return {
    phase: shallowReadonly(phase),
    error: shallowReadonly(error),
    preview: shallowReadonly(preview),
    handle: shallowReadonly(handle),
    sessionId: shallowReadonly(sessionId),
    parentSessionId: shallowReadonly(parentSessionId),
    mode: shallowReadonly(mode),
    status: shallowReadonly(status),
    events: shallowReadonly(events),
    liveEvents: shallowReadonly(liveEvents),
    noNewEvents: shallowReadonly(noNewEvents),
    tailPartial: shallowReadonly(tailPartial),
    tailDiagnostics: shallowReadonly(tailDiagnostics),
    tailError: shallowReadonly(tailError),
    tailCaughtUp: shallowReadonly(tailCaughtUp),
    preflight,
    start,
    write,
    resize,
    close,
  };
}
