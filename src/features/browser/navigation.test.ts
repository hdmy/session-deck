import { describe, expect, it } from 'vitest';
import type { Project, SearchHit, SessionSummary } from '../../types';
import { hitsBySession, projectsForQuery, sessionsForProvider, sortSessions } from './navigation';

function session(id: string, projectId: string): SessionSummary {
  return {
    id,
    provider_id: 'claude',
    project_id: projectId,
    title: id,
    source_title: id,
    hidden: false,
    pinned: false,
    last_used_at: null,
    started_at: null,
    ended_at: null,
    branch: null,
    first_prompt: null,
    last_prompt: null,
    cwd: null,
    models: [],
    tool_count: 0,
    source_mtime: 0,
    partial: false,
  };
}

function hiddenSession(id: string, provider_id: 'claude' | 'codex'): SessionSummary {
  return { ...session(id, 'hidden'), provider_id, hidden: true };
}

function project(id: string, sessions: SessionSummary[]): Project {
  return { id, name: id, path: `/tmp/${id}`, latest_activity: null, sessions };
}

function hit(value: SessionSummary, eventId: number): SearchHit {
  return { session: value, snippet: `match ${value.id}`, event_id: eventId };
}

describe('project search projection', () => {
  const a1 = session('a1', 'a');
  const a2 = session('a2', 'a');
  const b1 = session('b1', 'b');
  const projects = [project('a', [a1, a2]), project('b', [b1])];

  it('keeps the complete tree when the query is empty', () => {
    expect(projectsForQuery(projects, [], '')).toEqual(projects);
  });

  it('keeps matching sessions grouped under their projects', () => {
    expect(projectsForQuery(projects, [hit(a2, 2), hit(b1, 3)], 'cloud')).toEqual([
      project('a', [a2]),
      project('b', [b1]),
    ]);
  });

  it('uses the first hit for session navigation', () => {
    const first = hit(a1, 7);
    expect(hitsBySession([first, hit(a1, 9)]).get('a1')).toEqual(first);
  });

  it('sorts pinned sessions before recent activity and keeps stable id fallback', () => {
    const old = { ...a1, last_used_at: 10 };
    const recent = { ...a2, last_used_at: 30 };
    const pinned = { ...b1, pinned: true, last_used_at: 1 };
    expect(sortSessions([old, pinned, recent]).map((item) => item.id)).toEqual(['b1', 'a2', 'a1']);
  });

  it('treats last_used_at=0 as an explicit activity value, not as missing', () => {
    const explicitZero = { ...a1, id: 'explicit-zero', last_used_at: 0, ended_at: 999 };
    const fallbackToEnded = { ...a2, id: 'fallback-ended', last_used_at: null, ended_at: 10 };
    expect(sortSessions([fallbackToEnded, explicitZero]).map((item) => item.id)).toEqual([
      'fallback-ended',
      'explicit-zero',
    ]);
  });

  it('uses the session id when title and activity are tied', () => {
    const laterId = { ...a1, id: 'z-session', title: 'Same title', source_title: 'Same title', last_used_at: 42 };
    const earlierId = { ...a2, id: 'a-session', title: 'Same title', source_title: 'Same title', last_used_at: 42 };
    expect(sortSessions([laterId, earlierId]).map((item) => item.id)).toEqual(['a-session', 'z-session']);
  });
});

describe('hidden session provider projection', () => {
  const hidden = [hiddenSession('claude-hidden', 'claude'), hiddenSession('codex-hidden', 'codex')];

  it('keeps all hidden sessions for the all-provider filter', () => {
    expect(sessionsForProvider(hidden, null).map((item) => item.id)).toEqual(['claude-hidden', 'codex-hidden']);
  });

  it('derives only codex hidden sessions without mutating the source list', () => {
    expect(sessionsForProvider(hidden, 'codex').map((item) => item.id)).toEqual(['codex-hidden']);
    expect(hidden).toHaveLength(2);
  });

  it('derives only claude hidden sessions', () => {
    expect(sessionsForProvider(hidden, 'claude').map((item) => item.id)).toEqual(['claude-hidden']);
  });
});
