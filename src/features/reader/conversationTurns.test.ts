import { describe, expect, it } from 'vitest';
import type { ConversationTurn, TimelineEvent, TurnActivity } from '../../types';
import { buildConversationTurns, isFinalAssistantText } from './conversationTurns';

const event = (id: number, kind: TimelineEvent['kind'], content: string, final_response = false): TimelineEvent => ({ id, session_id: 'fixture', kind, role: kind === 'assistant' || kind === 'user' ? kind : null, content, timestamp: id, tool_name: null, collapsed: kind !== 'user' && kind !== 'assistant', final_response });
const activity = (event_id: number, kind: TurnActivity['kind'], content: string, final_response = false): TurnActivity => ({ event_id, kind, role: kind === 'assistant' || kind === 'user' ? kind : null, content, timestamp: event_id, tool_name: null, tool_use_id: null, parent_tool_use_id: null, collapsed: kind !== 'assistant' && kind !== 'user', final_response });

describe('conversation turns', () => {
  it('marks only the last non-empty assistant text before the next user', () => {
    const turns = buildConversationTurns([
      event(1, 'user', 'first'), event(2, 'assistant', 'draft'), event(3, 'tool_use', 'run'), event(4, 'assistant', ''), event(5, 'assistant', 'final'), event(6, 'user', 'second'), event(7, 'assistant', 'second final'),
    ]);
    expect(turns).toHaveLength(2);
    expect(turns[0].finalAssistant?.id).toBe(5);
    expect(isFinalAssistantText(turns[0].orderedEvents[1], turns[0])).toBe(false);
    expect(isFinalAssistantText(turns[0].orderedEvents[4], turns[0])).toBe(true);
  });

  it('prefers backend turns and preserves activity order without duplicating user activity', () => {
    const backend: ConversationTurn = { id: 4, session_id: 'native', user_prompt: 'prompt', timestamp: 1, completed: true, final_response: 'done', activities: [activity(10, 'user', 'prompt'), activity(11, 'tool_use', 'run'), activity(12, 'assistant', 'draft'), activity(13, 'tool_result', 'result'), activity(14, 'assistant', 'done', true)] };
    const turns = buildConversationTurns({ timeline: [], turns: [backend] });
    expect(turns[0].orderedEvents.map((item) => item.kind)).toEqual(['user', 'tool_use', 'assistant', 'tool_result', 'assistant']);
    expect(turns[0].orderedEvents.filter((item) => item.kind === 'user')).toHaveLength(1);
    expect(turns[0].orderedEvents[turns[0].orderedEvents.length - 1]?.final_response).toBe(true);
  });

  it('keeps activity in a pending turn while completed stays false', () => {
    const turns = buildConversationTurns([event(1, 'user', 'prompt'), event(2, 'thinking', 'plan')]);
    expect(turns[0].focusActivities).toHaveLength(1);
    expect(turns[0].completed).toBe(false);
    expect(turns[0].finalAssistant).toBeNull();
  });

  it('maps a normalized user turn to its real timeline event id', () => {
    const backend: ConversationTurn = { id: 4, session_id: 'native', user_prompt: 'prompt', timestamp: 1, completed: true, final_response: 'done', activities: [activity(11, 'assistant', 'done', true)] };
    const turns = buildConversationTurns({ timeline: [event(42, 'user', 'prompt'), event(11, 'assistant', 'done', true)], turns: [backend] });
    expect(turns[0].user?.id).toBe(42);
    expect(turns[0].orderedEvents[0]?.id).toBe(42);
  });
});
