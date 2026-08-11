import type { ContinuationViewState } from './continuationTypes';

/**
 * Return the fork target that should remain guarded for an observed runtime
 * state. A fork child is bound once; later observations cannot replace it.
 */
export function bindForkChild(
  currentMode: 'resume' | 'fork',
  sourceSessionId: string | null,
  targetSessionId: string | null,
  state: Pick<ContinuationViewState, 'mode' | 'sessionId' | 'parentSessionId'>,
): string | null {
  if (
    currentMode !== 'fork'
    || state.mode !== 'fork'
    || !sourceSessionId
    || !state.sessionId
    || state.sessionId === sourceSessionId
    || (state.parentSessionId !== null && state.parentSessionId !== sourceSessionId)
  ) return targetSessionId;

  // The initial fork target is the source parent. Once a real child is bound,
  // keep it stable even if a later poll reports another session id.
  if (targetSessionId && targetSessionId !== sourceSessionId) return targetSessionId;
  return state.sessionId;
}
