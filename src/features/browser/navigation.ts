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
    const scopedId = `${hit.session.provider_id}:${hit.session.id}`;
    if (!index.has(scopedId)) index.set(scopedId, hit);
    if (!index.has(hit.session.id)) index.set(hit.session.id, hit);
  }
  return index;
}

export function projectsForQuery(
  projects: readonly Project[],
  hits: readonly SearchHit[],
  query: string,
  providerId: string | null = null,
): Project[] {
  const sessionIds = new Set(hits.map((hit) => hit.session.id));
  const visible = (project: Project): Project => {
    const providerSessions = project.sessions.filter((session) => !providerId || session.provider_id === providerId);
    const sessions = query.trim()
      ? providerSessions.filter((session) => sessionIds.has(session.id))
      : providerSessions;
    const agents = project.agents
      ?.map((agent) => ({
        ...agent,
        sessions: sortSessions(agent.sessions.filter((session) =>
          (!providerId || agent.provider_id === providerId) &&
          (!query.trim() || sessionIds.has(session.id)),
        )),
      }))
      .filter((agent) => agent.sessions.length > 0);
    return { ...project, sessions: sortSessions(sessions), ...(agents ? { agents } : {}) };
  };

  return projects
    .map(visible)
    .filter((project) => project.sessions.length > 0);
}

export function sessionsForProvider(
  sessions: readonly SessionSummary[],
  providerId: string | null,
): SessionSummary[] {
  return providerId === null
    ? [...sessions]
    : sessions.filter((session) => session.provider_id === providerId);
}
