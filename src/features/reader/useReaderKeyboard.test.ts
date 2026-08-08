import { createApp, shallowRef } from 'vue';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useReaderKeyboard } from './useReaderKeyboard';

describe('useReaderKeyboard IME guard', () => {
  let app: ReturnType<typeof createApp> | null = null;
  let host: HTMLDivElement | null = null;

  afterEach(() => {
    app?.unmount();
    host?.remove();
    app = null;
    host = null;
  });

  it.each([
    ['composition event', { isComposing: true }],
    ['IME keyCode 229', { keyCode: 229 }],
  ])('does not intercept find for %s', async (_label, overrides) => {
    const open = vi.fn();
    host = document.createElement('div');
    document.body.append(host);
    app = createApp({
      setup() {
        useReaderKeyboard({ input: shallowRef(null), onOpen: open, onNext: vi.fn(), onPrevious: vi.fn(), onClose: vi.fn() });
        return {};
      },
      template: '<div />',
    });
    app.mount(host);
    const event = new KeyboardEvent('keydown', { key: 'f', ctrlKey: true, bubbles: true });
    for (const [key, value] of Object.entries(overrides)) Object.defineProperty(event, key, { configurable: true, value });
    window.dispatchEvent(event);
    expect(open).not.toHaveBeenCalled();
  });
});
