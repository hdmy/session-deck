import { createApp, nextTick } from 'vue';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SessionSummary } from '../../types';
import HiddenSessionsPanel from './HiddenSessionsPanel.vue';

const roots: HTMLElement[] = [];
function session(id: string): SessionSummary {
  return {
    id, provider_id: 'claude', project_id: 'project', title: id, source_title: id, hidden: true,
    pinned: false, last_used_at: null, started_at: 1, ended_at: 2, branch: 'main', first_prompt: null,
    last_prompt: null, cwd: '/tmp/project', models: [], tool_count: 0, source_mtime: 1, partial: false,
  };
}

afterEach(() => { for (const root of roots.splice(0)) root.remove(); });

describe('HiddenSessionsPanel continuation lock', () => {
  it('locks active parent and child restores but leaves other sessions operable', async () => {
    const root = document.createElement('div');
    document.body.append(root); roots.push(root);
    const restore = vi.fn();
    createApp(HiddenSessionsPanel, {
      sessions: [session('parent'), session('child'), session('other')],
      loading: false,
      error: null,
      activeContinuationSessionId: 'parent',
      activeContinuationTargetSessionId: 'child',
      onRestore: restore,
    }).mount(root);
    await nextTick();
    root.querySelector<HTMLButtonElement>('.project-head')!.click();
    await nextTick();
    const buttons = [...root.querySelectorAll<HTMLButtonElement>('.restore-button')];
    expect(buttons[0]?.disabled).toBe(true);
    expect(buttons[1]?.disabled).toBe(true);
    expect(buttons[2]?.disabled).toBe(false);
    expect(root.textContent).toContain('parent session');
    expect(root.textContent).toContain('fork child');
    buttons[2]?.click();
    expect(restore).toHaveBeenCalledWith('other');
  });
});
