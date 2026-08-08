import { afterEach, describe, expect, it, vi } from 'vitest';
import type { MockInstance } from 'vitest';
import { copyText, fallbackCopyText } from './copyText';

afterEach(() => vi.restoreAllMocks());

function mockExecCommand(result: boolean | (() => boolean)): MockInstance<typeof document.execCommand> {
  Object.defineProperty(document, 'execCommand', { configurable: true, value: typeof result === 'function' ? result : () => result });
  return vi.spyOn(document, 'execCommand');
}

describe('copyText', () => {
  it('prefers the async clipboard', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    const exec = mockExecCommand(false);
    expect(await copyText('hello')).toBe(true);
    expect(writeText).toHaveBeenCalledWith('hello');
    expect(exec).not.toHaveBeenCalled();
  });

  it('falls back when clipboard is missing or rejects', async () => {
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: undefined });
    mockExecCommand(true);
    expect(await copyText('fallback')).toBe(true);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: vi.fn().mockRejectedValue(new Error('denied')) } });
    expect(await copyText('fallback after reject')).toBe(true);
  });

  it('reports execCommand failures and always removes the textarea', () => {
    mockExecCommand(() => { throw new Error('unsupported'); });
    expect(fallbackCopyText('error')).toBe(false);
    expect(document.querySelectorAll('textarea')).toHaveLength(0);
  });
});
