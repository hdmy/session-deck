import type { ContinuationEvent } from '../../types';

export function continuationFinished(status: { handle?: string; status: string; events: ContinuationEvent[] }): boolean {
  // An `exited` event only says that the PTY waiter observed process exit. The
  // runtime can still be draining buffered PTY/tail events, so status is the
  // authoritative completion signal. In particular, keep polling while the
  // backend reports `running` or `draining` even when an Exited event is in the
  // same response.
  const value = status.status.trim().toLowerCase();
  if (
    value === '' ||
    value === 'running' ||
    value === 'draining' ||
    value === 'starting' ||
    value === 'preflighting' ||
    value.startsWith('started ')
  ) {
    return false;
  }
  return true;
}

export function encodeTerminalInput(value: string): number[] {
  return Array.from(new TextEncoder().encode(value));
}

export function isContinuationStale(token: number, currentGeneration: number, disposed: boolean): boolean {
  return disposed || token !== currentGeneration;
}
