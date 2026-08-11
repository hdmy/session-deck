import { createApp, nextTick } from 'vue';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { RelatedSession } from '../../types';
import RelatedSessionsPanel from './RelatedSessionsPanel.vue';

const related: RelatedSession = {
  session: { id: 's2', provider_id: 'claude', project_id: 'p', title: 'Related', source_title: 'Related', hidden: false, pinned: false, last_used_at: null, started_at: 1, ended_at: null, branch: null, first_prompt: null, last_prompt: null, cwd: null, models: [], tool_count: 0, source_mtime: 1, partial: false },
  relation_type: 'solution', score: 0.8, reason: 'same fix',
};

describe('RelatedSessionsPanel', () => {
  let app: ReturnType<typeof createApp> | undefined;
  let host: HTMLDivElement | undefined;
  afterEach(() => { app?.unmount(); host?.remove(); });

  it('renders related session and emits open-session', async () => {
    const open = vi.fn(); host = document.createElement('div'); document.body.append(host);
    app = createApp(RelatedSessionsPanel, { related: [related], loading: false, error: null, onOpenSession: open }); app.mount(host);
    await nextTick();
    expect(host.textContent).toContain('Related');
    host.querySelector<HTMLButtonElement>('.related-session')!.click();
    expect(open).toHaveBeenCalledWith('s2');
  });
});
