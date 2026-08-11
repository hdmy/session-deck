import type { LiveTranscriptEvent } from '../../types';
import type { TerminalPhase } from './useContinuationTerminal';

export interface ContinuationViewState {
  sessionId: string;
  parentSessionId: string | null;
  mode: 'resume' | 'fork';
  phase: TerminalPhase;
  status: string | null;
  liveEvents: readonly LiveTranscriptEvent[];
  noNewEvents?: boolean;
  tailPartial: boolean;
  tailDiagnostics: number;
  tailError: string | null;
  tailCaughtUp: boolean;
  error: string | null;
}
