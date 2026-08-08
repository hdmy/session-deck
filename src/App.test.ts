import { createApp, nextTick } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BranchSummary, Project, ScanReport, SessionDetail, SessionSummary } from './types';
import { bindForkChild } from './features/terminal/continuationChildBinding';

const mocks = vi.hoisted(() => ({
  scan: vi.fn(), projects: vi.fn(), session: vi.fn(), listHiddenSessions: vi.fn(), search: vi.fn(), touchSession: vi.fn(),
  getSessionBranch: vi.fn(), setSessionHidden: vi.fn(), renameSession: vi.fn(), setSessionPinned: vi.fn(),
  startContinuation: vi.fn(), startForkContinuation: vi.fn(), pollContinuation: vi.fn(), closeContinuation: vi.fn(),
  writeContinuation: vi.fn(), resizeContinuation: vi.fn(), resumePreflight: vi.fn(),
  getScanSettings: vi.fn(),
}));

vi.mock('./api', () => ({ api: mocks }));

const xterm = vi.hoisted(() => {
  class FakeTerminal {
    rows = 24;
    cols = 80;
    loadAddon = vi.fn();
    open = vi.fn();
    onData = vi.fn();
    dispose = vi.fn();
    write = vi.fn();
  }
  class FakeFitAddon { fit = vi.fn(); }
  return { FakeTerminal, FakeFitAddon };
});
vi.mock('@xterm/xterm', () => ({ Terminal: xterm.FakeTerminal }));
vi.mock('@xterm/addon-fit', () => ({ FitAddon: xterm.FakeFitAddon }));

import App from './App.vue';

function summary(id: string, title = id): SessionSummary {
  return {
    id, native_session_id: `native-${id}`, provider_id: 'claude', project_id: 'project-1', title, source_title: title,
    hidden: false, pinned: false, last_used_at: null, started_at: 1, ended_at: null, branch: 'main',
    first_prompt: 'hello', last_prompt: 'hello', cwd: '/tmp/project', models: ['claude'], tool_count: 0,
    source_mtime: 1, partial: false,
  };
}
function project(sessions: SessionSummary[]): Project {
  return { id: 'project-1', name: 'project-1', path: '/tmp/project', latest_activity: 1, sessions };
}
function branch(): BranchSummary {
  return { id: 'main', session_id: 'parent', label: 'main', kind: 'active', root_uuid: null, leaf_uuid: null, fork_point_uuid: null, is_active: true, event_count: 1, turn_count: 1, started_at: 1, ended_at: null, compacted: false };
}
function detail(id: string): SessionDetail {
  return { summary: summary(id), timeline: [], diagnostics: [], branches: [branch()], active_branch_id: 'main', selected_branch_id: 'main' };
}
function report(): ScanReport { return { root: '/tmp', trigger: 'manual', outcome: 'committed', committed: true, sessions: 1, diagnostics: 0, partial: false, removed_sessions: 0, new_files: 0, changed_files: 0, unchanged_files: 0, removed_files: 0, partial_sessions: 0 }; }
async function settle(): Promise<void> {
  for (let index = 0; index < 8; index += 1) {
    await nextTick();
    await Promise.resolve();
  }
}

describe('App fork child binding', () => {
  const forkState = (sessionId: string, parentSessionId: string | null = 'parent') => ({
    mode: 'fork' as const,
    sessionId,
    parentSessionId,
  });

  it('binds child once, rejects a later foreign id, and keeps same child idempotent', () => {
    const child = bindForkChild('fork', 'parent', 'parent', forkState('child', null));
    expect(child).toBe('child');
    expect(bindForkChild('fork', 'parent', child, forkState('foreign'))).toBe('child');
    expect(bindForkChild('fork', 'parent', child, forkState('child'))).toBe('child');
  });

  it('rejects a wrong parent and ignores fork state injected into resume', () => {
    expect(bindForkChild('fork', 'parent', 'parent', forkState('child', 'other'))).toBe('parent');
    expect(bindForkChild('resume', 'parent', 'parent', forkState('child'))).toBe('parent');
  });
});

describe('App continuation boundary', () => {
  let root: HTMLElement;
  let app: ReturnType<typeof createApp>;
  let releasePoll!: (value: any) => void;
  let pollPending: Promise<unknown>;
  let projectState: SessionSummary[];
  const parent = summary('parent', 'Parent');
  const child = summary('child', 'Child');

  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => { callback(0); return 1; });
    vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(() => undefined);
    mocks.scan.mockResolvedValue(report());
    // Keep the child visible for the guard/fork propagation tests; individual
    // missing-source tests narrow this list after the initial scan settles.
    projectState = [parent, child];
    mocks.projects.mockImplementation(async () => [project(projectState)]);
    mocks.listHiddenSessions.mockResolvedValue([]);
    mocks.search.mockResolvedValue([]);
    mocks.session.mockImplementation(async (id: string) => detail(id));
    mocks.getSessionBranch.mockImplementation(async (id: string) => detail(id));
    mocks.touchSession.mockImplementation(async (id: string) => id === 'child' ? child : summary(id));
    mocks.setSessionHidden.mockImplementation(async (id: string, hidden: boolean) => ({ ...summary(id), hidden }));
    mocks.renameSession.mockImplementation(async (id: string, title: string | null) => ({ ...summary(id), title: title ?? id }));
    mocks.setSessionPinned.mockImplementation(async (id: string, pinned: boolean) => ({ ...summary(id), pinned }));
    mocks.startContinuation.mockResolvedValue({ handle: 'h', session_id: 'parent', parent_session_id: null, status: 'running', events: [], live_events: [], tail_partial: false, tail_diagnostics: 0, tail_error: null, tail_caught_up: false });
    mocks.startForkContinuation.mockResolvedValue({ handle: 'fork-h', session_id: 'child', parent_session_id: 'parent', status: 'running', events: [], live_events: [], tail_partial: false, tail_diagnostics: 0, tail_error: null, tail_caught_up: false });
    mocks.closeContinuation.mockResolvedValue(undefined);
    mocks.writeContinuation.mockResolvedValue(undefined);
    mocks.resizeContinuation.mockResolvedValue(undefined);
    mocks.getScanSettings.mockResolvedValue({ source_root: null, effective_root: '/tmp', scan_interval_seconds: 0 });
    mocks.pollContinuation.mockImplementation(() => { pollPending = new Promise((resolve) => { releasePoll = resolve; }); return pollPending; });
    root = document.createElement('div');
    document.body.append(root);
    app = createApp(App);
    app.mount(root);
  });

  afterEach(() => {
    app.unmount();
    root.remove();
    vi.restoreAllMocks();
  });

  async function openFork(): Promise<void> {
    await settle();
    [...root.querySelectorAll<HTMLButtonElement>('.session-item')]
      .find((button) => button.textContent?.includes('Parent'))?.click();
    await settle();
    root.querySelector<HTMLButtonElement>('.fork-button')?.click();
    await settle();
  }

  it('propagates fork child state and refreshes/selects the child after close', async () => {
    projectState = [parent, child];
    await openFork();
    expect(root.textContent).toContain('Child');
    expect(root.querySelector('.continuation-dock')).not.toBeNull();
    releasePoll({ handle: 'fork-h', session_id: 'child', parent_session_id: 'parent', status: 'exited', events: [], live_events: [], tail_partial: false, tail_diagnostics: 0, tail_error: null, tail_caught_up: true });
    await settle();
    expect(mocks.scan).toHaveBeenCalledTimes(2);
    expect(mocks.session).toHaveBeenCalledWith('child');
    expect(root.querySelector('.session-item-row.active strong')?.textContent).toContain('Child');
  });

  it('keeps the first child when a later foreign poll arrives, then selects that child on exit', async () => {
    const foreign = summary('foreign', 'Foreign');
    projectState = [parent, child, foreign];
    vi.useFakeTimers();
    try {
      mocks.pollContinuation.mockReset();
      mocks.pollContinuation
        .mockResolvedValueOnce({ handle: 'fork-h', session_id: 'child', parent_session_id: null, status: 'running', events: [], live_events: [], tail_partial: false, tail_diagnostics: 0, tail_error: null, tail_caught_up: false })
        .mockResolvedValueOnce({ handle: 'fork-h', session_id: 'foreign', parent_session_id: 'parent', status: 'exited', events: [], live_events: [], tail_partial: false, tail_diagnostics: 0, tail_error: null, tail_caught_up: true });
      await openFork();
      await vi.advanceTimersByTimeAsync(120);
      await settle();
      await vi.advanceTimersByTimeAsync(120);
      await settle();
      expect(mocks.session).toHaveBeenCalledWith('child');
      expect(root.querySelector('.session-item-row.active strong')?.textContent).toContain('Child');
    } finally {
      vi.useRealTimers();
    }
  });

  it('shows scan errors after exit instead of reporting a missing child', async () => {
    await settle();
    projectState = [parent];
    mocks.scan.mockRejectedValueOnce(new Error('scan unavailable'));
    await openFork();
    releasePoll({ handle: 'fork-h', session_id: 'child', parent_session_id: 'parent', status: 'exited', events: [], live_events: [], tail_partial: false, tail_diagnostics: 0, tail_error: null, tail_caught_up: true });
    await settle();
    expect(root.textContent).toContain('扫描/同步未完成：scan unavailable');
    expect(root.textContent).not.toContain('未发现新的 source');
  });

  it('reports a missing child only after a successful scan confirms absence', async () => {
    await settle();
    projectState = [parent];
    await openFork();
    releasePoll({ handle: 'fork-h', session_id: 'child', parent_session_id: 'parent', status: 'exited', events: [], live_events: [], tail_partial: false, tail_diagnostics: 0, tail_error: null, tail_caught_up: true });
    await settle();
    expect(root.textContent).toContain('未发现新的 source');
  });

  it('guards parent and child mutations while a fork is active', async () => {
    projectState = [parent, child];
    await openFork();
    const hides = [...root.querySelectorAll<HTMLButtonElement>('.hide-action')];
    expect(hides.length).toBe(2);
    expect(hides.map((button) => button.disabled)).toEqual([true, true]);
    expect(root.querySelector<HTMLButtonElement>('.refresh-button')?.disabled).toBe(true);
  });

  it('keeps the dock and parent guard when close is rejected', async () => {
    mocks.startForkContinuation.mockResolvedValue({ handle: 'fork-h', session_id: 'parent', parent_session_id: 'parent', status: 'running', events: [], live_events: [], tail_partial: false, tail_diagnostics: 0, tail_error: null, tail_caught_up: false });
    await openFork();
    mocks.closeContinuation.mockRejectedValueOnce(new Error('close denied'));
    root.querySelector<HTMLButtonElement>('.terminal-controls button:last-child')?.click();
    await settle();
    expect(root.querySelector('.continuation-dock')).not.toBeNull();
    expect([...root.querySelectorAll<HTMLButtonElement>('.hide-action')].some((button) => button.disabled)).toBe(true);
    expect(root.querySelector('.refresh-button')?.textContent).toContain('unavailable');
  });

  it('shows an explicit automatic scan settings error', async () => {
    app.unmount();
    mocks.getScanSettings.mockRejectedValue(new Error('settings unavailable'));
    app = createApp(App);
    app.mount(root);
    await settle();
    expect(root.textContent).toContain('自动扫描设置不可用：settings unavailable');
  });
});
