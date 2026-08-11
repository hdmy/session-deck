import { createApp, nextTick } from 'vue';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { RelatedSession } from '../../types';
import SemanticSearchPanel from './SemanticSearchPanel.vue';

const result: RelatedSession = {
  session: { id: 's2', provider_id: 'codex', project_id: 'p', title: 'Result title', source_title: 'Result title', hidden: false, pinned: false, last_used_at: null, started_at: 1, ended_at: null, branch: null, first_prompt: null, last_prompt: null, cwd: null, models: [], tool_count: 0, source_mtime: 1, partial: false },
  relation_type: 'semantic', score: 0.88, reason: 'related reason', summary: 'Result summary', topics: ['search'], tags: ['important'],
};

describe('SemanticSearchPanel', () => {
  let app: ReturnType<typeof createApp> | undefined;
  let host: HTMLDivElement | undefined;
  afterEach(() => { app?.unmount(); host?.remove(); });

  it('provides a visible query/scope entry and emits search on Enter/open', async () => {
    const search = vi.fn(); const open = vi.fn();
    host = document.createElement('div'); document.body.append(host);
    app = createApp(SemanticSearchPanel, { query: 'needle', scope: 'all', loading: false, error: null, results: [result], onSearch: search, onOpenSession: open }); app.mount(host);
    await nextTick();
    expect(host.querySelector('input[aria-label="Semantic search query"]')).not.toBeNull();
    expect(host.textContent).toContain('Result summary');
    host.querySelector<HTMLFormElement>('form')!.dispatchEvent(new SubmitEvent('submit', { bubbles: true }));
    host.querySelector<HTMLButtonElement>('.semantic-result')!.click();
    expect(search).toHaveBeenCalled();
    expect(open).toHaveBeenCalledWith('s2');
  });

  it('renders loading, error, and empty states', async () => {
    host = document.createElement('div'); document.body.append(host);
    app = createApp(SemanticSearchPanel, { query: '', scope: 'current', loading: true, error: null, results: [] }); app.mount(host);
    expect(host.textContent).toContain('Searching knowledge');
    app.unmount();
    app = createApp(SemanticSearchPanel, { query: '', scope: 'all', loading: false, error: 'failed', results: [] }); app.mount(host);
    expect(host.textContent).toContain('failed');
    app.unmount();
    app = createApp(SemanticSearchPanel, { query: '', scope: 'all', loading: false, error: null, results: [] }); app.mount(host);
    expect(host.textContent).toContain('Enter a keyword to search local history');
    app.unmount();
    app = createApp(SemanticSearchPanel, { query: 'needle', scope: 'all', loading: false, error: null, results: [] }); app.mount(host);
    expect(host.textContent).toContain('No semantic matches');
  });
});
