import type { ConversationTurn, SessionDetail, TimelineEvent, TurnActivity } from '../../types';

export interface ConversationViewTurn {
  id: string;
  user: TimelineEvent | null;
  orderedEvents: TimelineEvent[];
  focusActivities: TimelineEvent[];
  finalAssistant: TimelineEvent | null;
  completed: boolean;
}

export type ConversationSource = Pick<SessionDetail, 'timeline' | 'turns'> | TimelineEvent[];

function eventFromActivity(activity: TurnActivity, sessionId: string): TimelineEvent {
  return {
    id: activity.event_id,
    session_id: sessionId,
    kind: activity.kind as TimelineEvent['kind'],
    role: activity.role,
    content: activity.content,
    timestamp: activity.timestamp,
    tool_name: activity.tool_name,
    collapsed: activity.collapsed,
    tool_use_id: activity.tool_use_id,
    parent_tool_use_id: activity.parent_tool_use_id,
    final_response: activity.final_response,
  };
}

function userEvent(turn: ConversationTurn, source?: TimelineEvent): TimelineEvent | null {
  if (!turn.user_prompt?.trim()) return null;
  if (source) return { ...source, kind: 'user', role: source.role ?? 'user', content: turn.user_prompt };
  return {
    id: -Math.abs(turn.id),
    session_id: turn.session_id,
    kind: 'user',
    role: 'user',
    content: turn.user_prompt,
    timestamp: turn.timestamp,
    tool_name: null,
    collapsed: false,
    turn_id: turn.id,
  };
}

function fromBackendTurn(turn: ConversationTurn, sourceUser?: TimelineEvent): ConversationViewTurn {
  const user = userEvent(turn, sourceUser);
  const activities = turn.activities.map((activity) => eventFromActivity(activity, turn.session_id));
  // The backend retains the user activity in its activity list for lineage. It is
  // represented once by user_prompt in the reader, never twice in the UI.
  const orderedEvents = activities.filter((event) => event.kind !== 'user');
  const finalAssistant = orderedEvents.find((event) => event.final_response) ?? [...orderedEvents].reverse().find((event) => event.kind === 'assistant' && event.content.trim()) ?? null;
  return {
    id: `turn-${turn.id}`,
    user,
    orderedEvents: user ? [user, ...orderedEvents] : orderedEvents,
    focusActivities: orderedEvents.filter((event) => event.id !== finalAssistant?.id),
    finalAssistant,
    completed: turn.completed && finalAssistant !== null,
  };
}

function fromTimeline(events: TimelineEvent[]): ConversationViewTurn[] {
  const turns: ConversationViewTurn[] = [];
  let current: TimelineEvent[] = [];
  const flush = () => {
    if (!current.length) return;
    const user = current.find((event) => event.kind === 'user' && event.content.trim()) ?? null;
    const sourceEvents = [...current];
    const selectedFinal = [...sourceEvents].reverse().find((event) => event.kind === 'assistant' && event.content.trim()) ?? null;
    const orderedEvents = sourceEvents.map((event) => event.id === selectedFinal?.id ? { ...event, final_response: true } : event);
    const finalAssistant = orderedEvents.find((event) => event.final_response) ?? null;
    turns.push({ id: `turn-${user?.id ?? orderedEvents[0].id}`, user, orderedEvents, focusActivities: orderedEvents.filter((event) => event.kind !== 'user' && event.id !== finalAssistant?.id), finalAssistant, completed: finalAssistant !== null });
    current = [];
  };
  for (const event of events) {
    if (event.kind === 'user' && event.content.trim()) flush();
    current.push(event);
  }
  flush();
  return turns;
}

/** Prefer the provider's normalized turns; keep timeline grouping only for old indexes. */
export function buildConversationTurns(source: ConversationSource): ConversationViewTurn[] {
  if (!Array.isArray(source) && source.turns?.length) {
    const userEvents = source.timeline.filter((event) => event.kind === 'user' && event.content.trim());
    return source.turns.map((turn) => {
      // Normalized turns do not carry the user row id themselves. Recover it
      // from the timeline so global search hits can target the actual DOM id
      // instead of the synthetic negative turn id used by old indexes.
      const sourceUser = userEvents.find((event) => event.turn_id === turn.id && event.content === turn.user_prompt)
        ?? userEvents.find((event) => event.content === turn.user_prompt);
      return fromBackendTurn(turn, sourceUser);
    });
  }
  return fromTimeline(Array.isArray(source) ? source : source.timeline);
}

export function isFinalAssistantText(event: TimelineEvent, turn: ConversationViewTurn): boolean {
  return turn.finalAssistant?.id === event.id && event.kind === 'assistant' && event.content.trim().length > 0;
}
