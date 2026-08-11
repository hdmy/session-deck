import { effectScope } from 'vue';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { continuationFinished, encodeTerminalInput, isContinuationStale } from './continuationLogic';

const mocks = vi.hoisted(() => ({
  startContinuation: vi.fn(), pollContinuation: vi.fn(), closeContinuation: vi.fn(), writeContinuation: vi.fn(), resizeContinuation: vi.fn(), resumePreflight: vi.fn(),
}));
vi.mock('../../api', () => ({ api: mocks }));

import { useContinuationTerminal } from './useContinuationTerminal';

describe('continuation logic', () => {
  beforeEach(() => { vi.clearAllMocks(); mocks.closeContinuation.mockResolvedValue(undefined); });
  it('only finishes on a terminal status, not an Exited event while draining', () => {
    expect(continuationFinished({ handle: 'h', status: 'running', events: [] })).toBe(false);
    expect(continuationFinished({ handle: 'h', status: 'draining', events: [{ kind: 'exited', data: null, status: '0', message: null }] })).toBe(false);
    expect(continuationFinished({ handle: 'h', status: 'exited', events: [] })).toBe(true);
    expect(continuationFinished({ handle: 'h', status: 'running', events: [{ kind: 'exited', data: null, status: '0', message: null }] })).toBe(false);
  });
  it('encodes terminal input as UTF-8 bytes', () => { expect(encodeTerminalInput('a你')).toEqual([97, 228, 189, 160]); });
  it('identifies stale work after a generation change or scope disposal', () => { expect(isContinuationStale(1, 2, false)).toBe(true); expect(isContinuationStale(1, 1, true)).toBe(true); });

  it('best-effort closes the handle when polling fails', async () => {
    mocks.startContinuation.mockResolvedValue({ handle: 'h', status: 'started', events: [], tail_caught_up: false });
    mocks.pollContinuation.mockRejectedValue(new Error('poll down'));
    const scope = effectScope(); let controller!: ReturnType<typeof useContinuationTerminal>;
    scope.run(() => { controller = useContinuationTerminal(); });
    await controller.start('session');
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(mocks.closeContinuation).toHaveBeenCalledWith('h');
    expect(controller.phase.value).toBe('error');
    expect(controller.handle.value).toBeNull();
    expect(await controller.close()).toEqual({ status: 'closed' });
    scope.stop();
  });

  it('allows Close after start rejects without returning a handle', async () => {
    mocks.startContinuation.mockRejectedValueOnce(new Error('start denied'));
    const finished = vi.fn();
    const scope = effectScope(); let controller!: ReturnType<typeof useContinuationTerminal>;
    scope.run(() => { controller = useContinuationTerminal({ onFinished: finished }); });
    await controller.start('session');
    expect(controller.phase.value).toBe('error');
    expect(await controller.close()).toEqual({ status: 'closed' });
    expect(finished).toHaveBeenCalledWith('closed', 'closed');
    scope.stop();
  });

  it('keeps a pending start guarded until late-handle cleanup succeeds', async () => {
    let resolveStart!: (value: { handle: string; status: string; events: []; tail_caught_up: boolean }) => void;
    mocks.startContinuation.mockReturnValueOnce(new Promise((resolve) => { resolveStart = resolve; }));
    const scope = effectScope(); let controller!: ReturnType<typeof useContinuationTerminal>;
    scope.run(() => { controller = useContinuationTerminal(); });
    const pending = controller.start('session');
    expect(await controller.close()).toEqual({ status: 'error', error: 'Unable to close terminal: no active continuation handle.' });
    resolveStart({ handle: 'late', status: 'started', events: [], tail_caught_up: false });
    mocks.closeContinuation.mockResolvedValueOnce(undefined);
    await pending;
    expect(mocks.closeContinuation).toHaveBeenCalledWith('late');
    expect(controller.handle.value).toBeNull();
    expect(await controller.close()).toEqual({ status: 'closed' });
    scope.stop();
  });

  it('retains a late handle when cleanup rejects so Close can retry', async () => {
    let resolveStart!: (value: { handle: string; status: string; events: []; tail_caught_up: boolean }) => void;
    mocks.startContinuation.mockReturnValueOnce(new Promise((resolve) => { resolveStart = resolve; }));
    const scope = effectScope(); let controller!: ReturnType<typeof useContinuationTerminal>;
    scope.run(() => { controller = useContinuationTerminal(); });
    const pending = controller.start('session');
    await controller.close();
    mocks.closeContinuation.mockRejectedValueOnce(new Error('late cleanup denied'));
    resolveStart({ handle: 'late', status: 'started', events: [], tail_caught_up: false });
    await pending;
    expect(controller.handle.value).toBe('late');
    mocks.closeContinuation.mockResolvedValueOnce(undefined);
    expect(await controller.close()).toEqual({ status: 'closed' });
    expect(controller.handle.value).toBeNull();
    scope.stop();
  });

  it('closes a handle returned after the scope became stale', async () => {
    let resolveStart!: (value: { handle: string; status: string; events: []; tail_caught_up: boolean }) => void;
    mocks.startContinuation.mockReturnValue(new Promise((resolve) => { resolveStart = resolve; }));
    const scope = effectScope(); let controller!: ReturnType<typeof useContinuationTerminal>;
    scope.run(() => { controller = useContinuationTerminal(); });
    const pending = controller.start('session');
    scope.stop();
    resolveStart({ handle: 'late', status: 'started', events: [], tail_caught_up: false });
    await pending;
    expect(mocks.closeContinuation).toHaveBeenCalledWith('late');
  });

  it('keeps polling after Exited until the backend status is terminal', async () => {
    vi.useFakeTimers();
    mocks.startContinuation.mockResolvedValue({ handle: 'h', status: 'started (test)', events: [], live_events: [], tail_partial: false, tail_diagnostics: 0, tail_error: null, tail_caught_up: false });
    mocks.pollContinuation
      .mockResolvedValueOnce({
        handle: 'h', session_id: 'session', status: 'running',
        events: [{ kind: 'exited', data: null, status: '0', message: null }],
        live_events: [{ id: 'event-1', kind: 'assistant', role: 'assistant', content: 'draft', timestamp: 1, tool_name: null, collapsed: false }],
        tail_partial: true, tail_diagnostics: 3, tail_error: null, tail_caught_up: false,
      })
      .mockResolvedValueOnce({
        handle: 'h', session_id: 'session', status: 'exited', events: [], live_events: [],
        tail_partial: false, tail_diagnostics: 1, tail_error: null, tail_caught_up: true,
      });
    const scope = effectScope(); let controller!: ReturnType<typeof useContinuationTerminal>;
    scope.run(() => { controller = useContinuationTerminal(); });
    await controller.start('session');
    await vi.advanceTimersByTimeAsync(0);
    expect(controller.phase.value).toBe('running');
    expect(mocks.closeContinuation).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(120);
    expect(mocks.pollContinuation).toHaveBeenCalledTimes(2);
    expect(mocks.closeContinuation).toHaveBeenCalledWith('h');
    expect(controller.tailDiagnostics.value).toBe(3);
    expect(controller.tailPartial.value).toBe(false);
    expect(controller.tailCaughtUp.value).toBe(true);
    scope.stop();
    vi.useRealTimers();
  });

  it('replaces live events by id and still closes on a PTY error event', async () => {
    mocks.startContinuation.mockResolvedValue({ handle: 'h', status: 'started (test)', events: [], live_events: [], tail_partial: false, tail_diagnostics: 0, tail_error: null, tail_caught_up: false });
    mocks.pollContinuation.mockResolvedValue({
      handle: 'h', session_id: 'session', status: 'running',
      events: [{ kind: 'error', data: null, status: null, message: 'pty failed' }],
      live_events: [{ id: 1, kind: 'assistant', role: 'assistant', content: 'replacement', timestamp: 1, tool_name: null, collapsed: false }],
      tail_partial: false, tail_diagnostics: 0, tail_error: null, tail_caught_up: false,
    });
    const scope = effectScope(); let controller!: ReturnType<typeof useContinuationTerminal>;
    scope.run(() => { controller = useContinuationTerminal(); });
    await controller.start('session');
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(controller.phase.value).toBe('error');
    expect(controller.error.value).toBe('pty failed');
    expect(mocks.closeContinuation).toHaveBeenCalledWith('h');
    expect(controller.liveEvents.value).toEqual(expect.arrayContaining([expect.objectContaining({ id: 1, content: 'replacement' })]));
    scope.stop();
  });

  it('returns a close failure without releasing the handle or finished callback', async () => {
    mocks.startContinuation.mockResolvedValue({ handle: 'h', status: 'started (test)', events: [], live_events: [], tail_partial: false, tail_diagnostics: 0, tail_error: null, tail_caught_up: false });
    mocks.pollContinuation.mockResolvedValue({ handle: 'h', session_id: 'session', status: 'running', events: [], live_events: [], tail_partial: false, tail_diagnostics: 0, tail_error: null, tail_caught_up: false });
    const finished = vi.fn();
    const scope = effectScope(); let controller!: ReturnType<typeof useContinuationTerminal>;
    scope.run(() => { controller = useContinuationTerminal({ onFinished: finished }); });
    await controller.start('session');
    await new Promise((resolve) => setTimeout(resolve, 0));
    mocks.closeContinuation.mockRejectedValueOnce(new Error('close denied'));
    const failed = await controller.close();
    expect(failed).toEqual({ status: 'error', error: 'close denied' });
    expect(controller.handle.value).toBe('h');
    expect(controller.phase.value).toBe('error');
    expect(finished).not.toHaveBeenCalled();
    mocks.closeContinuation.mockResolvedValueOnce(undefined);
    const closed = await controller.close();
    expect(closed).toEqual({ status: 'closed' });
    expect(controller.handle.value).toBeNull();
    expect(finished).toHaveBeenCalledWith('closed', 'closed');
    scope.stop();
  });

  it('retains the handle when poll failure cleanup rejects', async () => {
    mocks.startContinuation.mockResolvedValue({ handle: 'h', status: 'started (test)', events: [], live_events: [], tail_partial: false, tail_diagnostics: 0, tail_error: null, tail_caught_up: false });
    mocks.pollContinuation.mockRejectedValue(new Error('poll down'));
    mocks.closeContinuation.mockRejectedValueOnce(new Error('close denied'));
    const scope = effectScope(); let controller!: ReturnType<typeof useContinuationTerminal>;
    scope.run(() => { controller = useContinuationTerminal(); });
    await controller.start('session');
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(controller.phase.value).toBe('error');
    expect(controller.handle.value).toBe('h');
    mocks.closeContinuation.mockResolvedValueOnce(undefined);
    expect(await controller.close()).toEqual({ status: 'closed' });
    scope.stop();
  });
});
