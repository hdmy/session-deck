import { createApp, h } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => {
  const state = <T>(value: T) => ({ value });
  const start = vi.fn();
  const controller = {
    phase: state<'idle'>('idle'), error: state<string | null>(null), status: state<string | null>(null),
    liveEvents: state<never[]>([]), noNewEvents: state(false), tailPartial: state(false), tailDiagnostics: state(0), tailError: state<string | null>(null),
    resize: vi.fn(), write: vi.fn(), start, close: vi.fn(),
  };
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
  return { start, controller, FakeTerminal, FakeFitAddon };
});

vi.mock('@xterm/xterm', () => ({ Terminal: mocks.FakeTerminal }));
vi.mock('@xterm/addon-fit', () => ({ FitAddon: mocks.FakeFitAddon }));
vi.mock('./useContinuationTerminal', () => ({ useContinuationTerminal: () => mocks.controller }));

import ContinuationTerminal from './ContinuationTerminal.vue';

describe('ContinuationTerminal lifecycle', () => {
  let root: HTMLElement;
  let app: ReturnType<typeof createApp>;
  let callbacks: Map<number, FrameRequestCallback>;
  let nextFrame: number;

  beforeEach(() => {
    root = document.createElement('div');
    document.body.append(root);
    app = createApp(ContinuationTerminal, { sessionId: 'session-1', title: 'Session' });
    callbacks = new Map();
    nextFrame = 0;
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => {
      const id = ++nextFrame;
      callbacks.set(id, callback);
      return id;
    });
    vi.spyOn(window, 'cancelAnimationFrame').mockImplementation((id) => { callbacks.delete(id); });
    mocks.start.mockClear();
    mocks.controller.close.mockReset();
  });

  afterEach(() => {
    app.unmount();
    root.remove();
    vi.restoreAllMocks();
  });

  it('cancels a pending start frame and never starts after unmount', () => {
    app.mount(root);
    expect(callbacks.size).toBeGreaterThan(0);
    app.unmount();
    for (const callback of callbacks.values()) callback(0);
    expect(mocks.start).not.toHaveBeenCalled();
  });

  it('emits closed when a recovered/start-rejected controller closes successfully', async () => {
    const closed = vi.fn();
    mocks.controller.close.mockResolvedValueOnce({ status: 'closed' });
    app = createApp({ render: () => h(ContinuationTerminal, { sessionId: 'session-1', title: 'Session', onClosed: closed }) });
    app.mount(root);
    await root.querySelector<HTMLButtonElement>('.terminal-controls button:last-child')?.click();
    await Promise.resolve();
    expect(closed).toHaveBeenCalledWith('closed');
  });

  it('keeps the dock mounted when controller close rejects', async () => {
    const closed = vi.fn();
    mocks.controller.close.mockResolvedValueOnce({ status: 'error', error: 'close denied' });
    app = createApp({ render: () => h(ContinuationTerminal, { sessionId: 'session-1', title: 'Session', onClosed: closed }) });
    app.mount(root);
    await root.querySelector<HTMLButtonElement>('.terminal-controls button:last-child')?.click();
    await Promise.resolve();
    expect(closed).not.toHaveBeenCalled();
    expect(root.querySelector('.continuation-dock')).not.toBeNull();
  });
});
