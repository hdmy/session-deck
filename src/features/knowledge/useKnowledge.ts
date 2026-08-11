import { onScopeDispose, shallowReadonly, shallowRef } from 'vue';
import { api } from '../../api';
import type { KnowledgeCard, KnowledgeCardPatch, ProviderId, RelatedSession } from '../../types';

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'An unexpected local error occurred.';
}

export function useKnowledge() {
  const card = shallowRef<KnowledgeCard | null>(null);
  const related = shallowRef<RelatedSession[]>([]);
  const loading = shallowRef(false);
  const saving = shallowRef(false);
  const error = shallowRef<string | null>(null);
  const activeSessionId = shallowRef<string | null>(null);
  const semanticQuery = shallowRef('');
  const semanticScope = shallowRef<'all' | 'current'>('all');
  const semanticResults = shallowRef<RelatedSession[]>([]);
  const semanticLoading = shallowRef(false);
  const semanticError = shallowRef<string | null>(null);
  let request = 0;
  let semanticRequest = 0;

  async function load(sessionId: string | null, _providerId?: ProviderId): Promise<void> {
    const currentRequest = ++request;
    activeSessionId.value = sessionId;
    semanticRequest += 1;
    semanticQuery.value = '';
    semanticResults.value = [];
    semanticError.value = null;
    semanticLoading.value = false;
    card.value = null;
    related.value = [];
    error.value = null;
    if (!sessionId) {
      loading.value = false;
      return;
    }
    if (typeof api.getKnowledgeCard !== 'function' || typeof api.relatedSessions !== 'function') {
      loading.value = false;
      return;
    }
    loading.value = true;
    try {
      const [nextCard, nextRelated] = await Promise.all([
        api.getKnowledgeCard(sessionId),
        // Related references are intentionally cross-agent by default.
        api.relatedSessions(sessionId, undefined),
      ]);
      if (currentRequest !== request) return;
      card.value = nextCard;
      related.value = nextRelated;
    } catch (cause) {
      if (currentRequest === request) error.value = errorMessage(cause);
    } finally {
      if (currentRequest === request) loading.value = false;
    }
  }

  function setSemanticQuery(value: string): void {
    semanticQuery.value = value;
  }

  function setSemanticScope(value: 'all' | 'current'): void {
    semanticScope.value = value;
  }

  async function searchSemantic(providerId?: ProviderId): Promise<void> {
    const query = semanticQuery.value.trim();
    const currentRequest = ++semanticRequest;
    semanticError.value = null;
    semanticResults.value = [];
    if (!query || !activeSessionId.value || typeof api.semanticSearch !== 'function') {
      semanticLoading.value = false;
      return;
    }
    semanticLoading.value = true;
    const scopedProvider = semanticScope.value === 'current' ? providerId : undefined;
    const sessionId = activeSessionId.value;
    try {
      const results = await api.semanticSearch(query, scopedProvider);
      if (currentRequest !== semanticRequest || sessionId !== activeSessionId.value) return;
      semanticResults.value = results;
    } catch (cause) {
      if (currentRequest === semanticRequest && sessionId === activeSessionId.value) semanticError.value = errorMessage(cause);
    } finally {
      if (currentRequest === semanticRequest) semanticLoading.value = false;
    }
  }

  async function save(patch: KnowledgeCardPatch): Promise<boolean> {
    if (saving.value) return false;
    const sessionId = activeSessionId.value;
    if (!sessionId) return false;
    const currentRequest = request;
    saving.value = true;
    error.value = null;
    try {
      const nextCard = await api.updateKnowledgeCard(sessionId, patch);
      if (currentRequest !== request || activeSessionId.value !== sessionId) return false;
      card.value = nextCard;
      return true;
    } catch (cause) {
      if (currentRequest === request) error.value = errorMessage(cause);
      return false;
    } finally {
      saving.value = false;
    }
  }

  onScopeDispose(() => { request += 1; });

  return {
    card: shallowReadonly(card),
    related: shallowReadonly(related),
    loading: shallowReadonly(loading),
    saving: shallowReadonly(saving),
    error: shallowReadonly(error),
    semanticQuery: shallowReadonly(semanticQuery),
    semanticScope: shallowReadonly(semanticScope),
    semanticResults: shallowReadonly(semanticResults),
    semanticLoading: shallowReadonly(semanticLoading),
    semanticError: shallowReadonly(semanticError),
    load,
    save,
    setSemanticQuery,
    setSemanticScope,
    searchSemantic,
  };
}
