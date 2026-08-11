import { effectScope } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { KnowledgeCard, RelatedSession, SessionSummary } from '../../types';

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    getKnowledgeCard: vi.fn(),
    relatedSessions: vi.fn(),
    updateKnowledgeCard: vi.fn(),
    semanticSearch: vi.fn(),
  },
}));
vi.mock('../../api', () => ({ api: apiMock }));

import { useKnowledge } from './useKnowledge';

function card(session_id: string): KnowledgeCard {
  return {
    session_id, title: `Card ${session_id}`, summary: 'Summary', topics: ['topic'], tags: ['tag'],
    decisions: [], troubleshooting: [], change_summary: '', body_markdown: 'body',
    source_session_ids: [session_id], auto_generated: true, updated_at: 1,
  };
}
function session(id: string): SessionSummary {
  return { id, provider_id: 'claude', project_id: 'p', title: id, source_title: id, hidden: false, pinned: false, last_used_at: null, started_at: 1, ended_at: null, branch: null, first_prompt: null, last_prompt: null, cwd: null, models: [], tool_count: 0, source_mtime: 1, partial: false };
}
function related(id: string): RelatedSession {
  return { session: session(id), relation_type: 'reference', score: 0.9, reason: 'same topic' };
}

describe('useKnowledge', () => {
  let scope: ReturnType<typeof effectScope>;
  beforeEach(() => {
    vi.clearAllMocks();
    apiMock.getKnowledgeCard.mockResolvedValue(card('one'));
    apiMock.relatedSessions.mockResolvedValue([related('related')]);
    apiMock.updateKnowledgeCard.mockResolvedValue(card('one'));
    apiMock.semanticSearch.mockResolvedValue([related('semantic')]);
    scope = effectScope();
  });
  afterEach(() => scope.stop());

  it('loads card and related sessions with loading state', async () => {
    let knowledge!: ReturnType<typeof useKnowledge>;
    scope.run(() => { knowledge = useKnowledge(); });
    const pending = knowledge.load('one', 'claude');
    expect(knowledge.loading.value).toBe(true);
    await pending;
    expect(knowledge.card.value?.session_id).toBe('one');
    expect(knowledge.related.value[0]?.session.id).toBe('related');
    expect(knowledge.loading.value).toBe(false);
  });

  it('ignores a stale session response', async () => {
    let releaseOne!: (value: KnowledgeCard) => void;
    apiMock.getKnowledgeCard.mockReturnValueOnce(new Promise((resolve) => { releaseOne = resolve; }));
    apiMock.getKnowledgeCard.mockResolvedValueOnce(card('two'));
    apiMock.relatedSessions.mockResolvedValue([]);
    let knowledge!: ReturnType<typeof useKnowledge>;
    scope.run(() => { knowledge = useKnowledge(); });
    const first = knowledge.load('one');
    await knowledge.load('two');
    releaseOne(card('one'));
    await first;
    expect(knowledge.card.value?.session_id).toBe('two');
  });

  it('surfaces load errors and saves a patch', async () => {
    let knowledge!: ReturnType<typeof useKnowledge>;
    scope.run(() => { knowledge = useKnowledge(); });
    apiMock.getKnowledgeCard.mockRejectedValueOnce(new Error('knowledge unavailable'));
    await knowledge.load('one');
    expect(knowledge.error.value).toBe('knowledge unavailable');
    apiMock.getKnowledgeCard.mockResolvedValueOnce(card('one'));
    await knowledge.load('one');
    await knowledge.save({ tags: ['updated'] });
    expect(apiMock.updateKnowledgeCard).toHaveBeenCalledWith('one', { tags: ['updated'] });
    expect(knowledge.card.value?.title).toBe('Card one');
  });

  it('does not apply a save response after switching sessions', async () => {
    let releaseSave!: (value: KnowledgeCard) => void;
    apiMock.updateKnowledgeCard.mockReturnValueOnce(new Promise((resolve) => { releaseSave = resolve; }));
    let knowledge!: ReturnType<typeof useKnowledge>;
    scope.run(() => { knowledge = useKnowledge(); });
    apiMock.getKnowledgeCard.mockReset();
    apiMock.getKnowledgeCard.mockResolvedValueOnce(card('one')).mockResolvedValueOnce(card('two'));
    await knowledge.load('one');
    const save = knowledge.save({ title: 'old' });
    await knowledge.load('two');
    releaseSave(card('one'));
    await save;
    expect(apiMock.updateKnowledgeCard).toHaveBeenCalledWith('one', { title: 'old' });
    expect(knowledge.card.value?.session_id).toBe('two');
  });

  it('searches all agents by default and scopes current provider explicitly', async () => {
    let knowledge!: ReturnType<typeof useKnowledge>;
    scope.run(() => { knowledge = useKnowledge(); });
    await knowledge.load('one', 'claude');
    knowledge.setSemanticQuery('  needle  ');
    await knowledge.searchSemantic('claude');
    expect(apiMock.semanticSearch).toHaveBeenLastCalledWith('needle', undefined);
    knowledge.setSemanticScope('current');
    await knowledge.searchSemantic('codex');
    expect(apiMock.semanticSearch).toHaveBeenLastCalledWith('needle', 'codex');
  });

  it('does not call semantic search for an empty query and clears on session change', async () => {
    let knowledge!: ReturnType<typeof useKnowledge>;
    scope.run(() => { knowledge = useKnowledge(); });
    await knowledge.load('one');
    knowledge.setSemanticQuery('   ');
    await knowledge.searchSemantic('claude');
    expect(apiMock.semanticSearch).not.toHaveBeenCalled();
    knowledge.setSemanticQuery('needle');
    await knowledge.searchSemantic('claude');
    expect(knowledge.semanticResults.value).toHaveLength(1);
    await knowledge.load('two');
    expect(knowledge.semanticResults.value).toEqual([]);
    expect(knowledge.semanticQuery.value).toBe('');
  });

  it('surfaces semantic errors and ignores stale results', async () => {
    let release!: (value: RelatedSession[]) => void;
    apiMock.semanticSearch.mockReturnValueOnce(new Promise((resolve) => { release = resolve; }));
    let knowledge!: ReturnType<typeof useKnowledge>;
    scope.run(() => { knowledge = useKnowledge(); });
    await knowledge.load('one');
    knowledge.setSemanticQuery('old');
    const stale = knowledge.searchSemantic('claude');
    await knowledge.load('two');
    release([related('old')]);
    await stale;
    expect(knowledge.semanticResults.value).toEqual([]);
    apiMock.semanticSearch.mockRejectedValueOnce(new Error('semantic unavailable'));
    knowledge.setSemanticQuery('new');
    await knowledge.searchSemantic();
    expect(knowledge.semanticError.value).toBe('semantic unavailable');
  });
});
