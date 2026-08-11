import type { SessionDetail, TimelineEvent } from '../../types';
import { buildConversationTurns, type ConversationViewTurn } from './conversationTurns';

export interface ReaderSearchMatch {
  eventId: number;
  start: number;
  end: number;
  text: string;
}

export interface ReaderSearchEvent {
  event: TimelineEvent;
  text: string;
}

export function displayedEvents(detail: SessionDetail, mode: 'focus' | 'full'): ReaderSearchEvent[] {
  const turns = buildConversationTurns(detail);
  if (!turns.length) return detail.timeline.map((event) => ({ event, text: event.content }));
  const events: TimelineEvent[] = [];
  for (const turn of turns) {
    const source = mode === 'full' ? turn.orderedEvents : [
      ...(turn.user ? [turn.user] : []),
      ...turn.focusActivities,
      ...(turn.finalAssistant ? [turn.finalAssistant] : []),
    ];
    events.push(...source);
  }
  return events.map((event) => ({ event, text: event.content }));
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

export function findReaderMatches(text: string, query: string, eventId = 0): ReaderSearchMatch[] {
  const trimmed = query.trim();
  if (!trimmed) return [];
  const matches: ReaderSearchMatch[] = [];
  const expression = new RegExp(escapeRegExp(trimmed), 'giu');
  let match: RegExpExecArray | null;
  while ((match = expression.exec(text))) {
    const start = match.index;
    matches.push({ eventId, start, end: start + match[0].length, text: match[0] });
    if (match[0].length === 0) expression.lastIndex += 1;
  }
  return matches;
}

export function searchReaderEvents(events: readonly ReaderSearchEvent[], query: string): ReaderSearchMatch[] {
  return events.flatMap(({ event, text }) => findReaderMatches(text, query, event.id));
}

export function nextMatchIndex(current: number, count: number): number {
  return count > 0 ? (current + 1 + count) % count : -1;
}

export function previousMatchIndex(current: number, count: number): number {
  return count > 0 ? (current - 1 + count) % count : -1;
}

export function scrollToReaderMatch(match: ReaderSearchMatch | undefined): boolean {
  if (!match || typeof document === 'undefined') return false;
  const element = document.getElementById(`event-${match.eventId}`);
  if (!element) return false;

  // Focus mode nests activity and tool events in collapsed disclosures. Open
  // only the disclosures between the match and this reader so a search hit is
  // actually visible without mutating unrelated page state.
  const readerBoundary = element.closest('.reader');
  let ancestor = element.parentElement;
  let openedDisclosure = false;
  while (ancestor && ancestor !== readerBoundary) {
    if (ancestor instanceof HTMLDetailsElement && !ancestor.open) {
      ancestor.open = true;
      openedDisclosure = true;
    }
    ancestor = ancestor.parentElement;
  }

  const scroll = () => element.scrollIntoView({ block: 'center', behavior: 'smooth' });
  // Scroll once now for callers that use this helper outside Vue, then repeat
  // after disclosure/render updates so the browser lays out the opened event.
  scroll();
  if (openedDisclosure && typeof window !== 'undefined') {
    const schedule = typeof window.requestAnimationFrame === 'function'
      ? window.requestAnimationFrame
      : (callback: FrameRequestCallback) => window.setTimeout(() => callback(Date.now()), 0);
    schedule(() => scroll());
  }
  return true;
}

/** Shared locator for navigation hits and in-reader search. */
export function scrollToReaderEvent(eventId: number): boolean {
  return scrollToReaderMatch({ eventId, start: 0, end: 0, text: '' });
}

// Kept as small aliases for callers that use the noun-first naming style.
export const getReaderMatches = searchReaderEvents;
export const getNextMatchIndex = nextMatchIndex;
export const getPreviousMatchIndex = previousMatchIndex;

export type { ConversationViewTurn };
