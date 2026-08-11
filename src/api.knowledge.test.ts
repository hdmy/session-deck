import { describe, expect, it, vi } from 'vitest';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { api } from './api';

describe('knowledge API payloads', () => {
  it('uses typed command payloads and optional provider filters', async () => {
    invoke.mockResolvedValue([]);
    await api.getKnowledgeCard('s1');
    expect(invoke).toHaveBeenLastCalledWith('get_knowledge_card', { sessionId: 's1' });
    await api.updateKnowledgeCard('s1', { tags: ['one'] });
    expect(invoke).toHaveBeenLastCalledWith('update_knowledge_card', { args: { sessionId: 's1', patch: { tags: ['one'] } } });
    await api.relatedSessions('s1', 'codex', 5);
    expect(invoke).toHaveBeenLastCalledWith('related_sessions', { args: { sessionId: 's1', providerId: 'codex', limit: 5 } });
    await api.semanticSearch('needle', undefined, 3);
    expect(invoke).toHaveBeenLastCalledWith('semantic_search', { args: { query: 'needle', limit: 3 } });
    await api.updateScanSettings({ scan_interval_seconds: 60, enabled_provider_ids: ['claude', 'codex'] });
    expect(invoke).toHaveBeenLastCalledWith('update_scan_settings', {
      update: { scanIntervalSeconds: 60, enabledProviderIds: ['claude', 'codex'] },
    });
  });
});
