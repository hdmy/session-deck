import { createApp, nextTick } from 'vue';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { KnowledgeCard } from '../../types';
import KnowledgeCardPanel from './KnowledgeCardPanel.vue';

const card: KnowledgeCard = {
  session_id: 's1', title: 'A card', summary: 'A summary', topics: ['topic'], tags: ['old'],
  decisions: ['keep it small'], troubleshooting: ['restart'], change_summary: 'changed',
  body_markdown: '# Body', source_session_ids: ['s1', 's2'], auto_generated: true, updated_at: 1,
};

describe('KnowledgeCardPanel', () => {
  let app: ReturnType<typeof createApp> | undefined;
  let host: HTMLDivElement | undefined;
  afterEach(() => { app?.unmount(); host?.remove(); });

  it('renders loading, error, and empty states', async () => {
    host = document.createElement('div'); document.body.append(host);
    app = createApp(KnowledgeCardPanel, { card: null, loading: true, error: null }); app.mount(host);
    expect(host.textContent).toContain('Loading knowledge');
    app.unmount();
    app = createApp(KnowledgeCardPanel, { card: null, loading: false, error: 'failed' }); app.mount(host);
    expect(host.textContent).toContain('failed');
    app.unmount();
    app = createApp(KnowledgeCardPanel, { card: null, loading: false, error: null }); app.mount(host);
    expect(host.textContent).toContain('No knowledge card');
  });

  it('edits tags and sources and emits a plain-text patch', async () => {
    const save = vi.fn();
    host = document.createElement('div'); document.body.append(host);
    app = createApp(KnowledgeCardPanel, { card, loading: false, error: null, onSave: save }); app.mount(host);
    await nextTick();
    expect(host.querySelector('.knowledge-title')).toBeNull();
    expect(host.querySelector('.knowledge-meta')).toBeNull();
    expect(host.querySelector('.knowledge-sources')).toBeNull();
    host.querySelector<HTMLButtonElement>('.knowledge-card-actions button')!.click();
    await nextTick();
    const textareas = [...host.querySelectorAll<HTMLTextAreaElement>('textarea')];
    textareas[1]!.value = 'new-tag\nsecond'; textareas[1]!.dispatchEvent(new Event('input', { bubbles: true }));
    textareas[2]!.value = 's3'; textareas[2]!.dispatchEvent(new Event('input', { bubbles: true }));
    host.querySelector<HTMLButtonElement>('.knowledge-card-actions button')!.click();
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ tags: ['new-tag', 'second'], source_session_ids: ['s3'] }));
    expect(host.innerHTML).not.toContain('v-html');
  });
});
