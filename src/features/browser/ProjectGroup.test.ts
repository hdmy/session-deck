import { createApp, nextTick } from 'vue';
import { afterEach, describe, expect, it } from 'vitest';
import ProjectGroup from './ProjectGroup.vue';
import type { Project, SessionSummary } from '../../types';

function session(): SessionSummary {
  return { id: 's', provider_id: 'claude', project_id: 'legacy', workspace_id: 'workspace', title: 'Session', source_title: 'Session', hidden: false, pinned: false, last_used_at: null, started_at: 1, ended_at: null, branch: null, first_prompt: null, last_prompt: null, cwd: '/tmp/worktree', models: [], tool_count: 0, source_mtime: 1, partial: false };
}

describe('ProjectGroup identity and alias controls', () => {
  let app: ReturnType<typeof createApp> | undefined;
  let host: HTMLDivElement | undefined;
  afterEach(() => { app?.unmount(); host?.remove(); });

  it('shows stable, cwd, and worktree paths and emits workspace alias reset from keyboard', async () => {
    const aliases: Array<[string, string | null]> = [];
    const project: Project = { id: 'legacy', workspace_id: 'workspace', name: 'Project', alias: 'Local name', path: '/repo', cwd_paths: ['/repo', '/repo/worktree'], worktree_paths: ['/repo/worktree'], latest_activity: 1, sessions: [session()] };
    host = document.createElement('div'); document.body.append(host);
    app = createApp(ProjectGroup, { project, selectedId: null, matches: new Map(), forceOpen: false, onAlias: (selection: [string, string | null]) => aliases.push(selection) });
    app.mount(host); await nextTick();
    expect(host.textContent).toContain('Local name');
    host.querySelector<HTMLButtonElement>('.path-toggle')!.click(); await nextTick();
    expect(host.textContent).toContain('/repo/worktree');
    host.querySelector<HTMLButtonElement>('.alias-action')!.click(); await nextTick();
    const input = host.querySelector<HTMLInputElement>('.alias-editor input')!;
    input.value = ''; input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); await nextTick();
    expect(aliases).toEqual([['workspace', null]]);
  });
});
