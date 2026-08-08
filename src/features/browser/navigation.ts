import type { Project, SearchHit, SessionSummary } from '../../types';

function activityTime(session: SessionSummary): number {
  return session.last_used_at ?? session.ended_at ?? session.started_at ?? session.source_mtime ?? 0;
}

/** Pinned sessions stay at the top; the rest follow the most recently used activity. */
export function sortSessions(sessions: readonly SessionSummary[]): SessionSummary[] {
  return [...sessions].sort((a, b) => {
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
    // Keep this lexical comparison locale-independent; SQLite orders session IDs by code unit.
    return activityTime(b) - activityTime(a) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  });
}

export function hitsBySession(hits: readonly SearchHit[]): ReadonlyMap<string, SearchHit> {
  const index = new Map<string, SearchHit>();
  for (const hit of hits) {
    if (!index.has(hit.session.id)) index.set(hit.session.id, hit);
  }
  return index;
}

export function projectsForQuery(
  projects: readonly Project[],
  hits: readonly SearchHit[],
  query: string,
): Project[] {
  if (!query.trim()) {
    return projects.map((project) => ({ ...project, sessions: sortSessions(project.sessions) }));
  }

  const sessionIds = new Set(hits.map((hit) => hit.session.id));
  return projects
    .map((project) => ({
      ...project,
      sessions: sortSessions(project.sessions.filter((session) => sessionIds.has(session.id))),
    }))
    .filter((project) => project.sessions.length > 0);
}
