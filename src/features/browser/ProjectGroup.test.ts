import { createApp, nextTick } from 'vue';
import { afterEach, describe, expect, it } from 'vitest';
import ProjectGroup from './ProjectGroup.vue';
import type { Project, SessionSummary } from '../../types';

function session(provider_id: 'claude' | 'codex' | 'opencode' = 'claude', id = 's'): SessionSummary {
  return { id, provider_id, project_id: 'legacy', workspace_id: 'workspace', title: 'Session', source_title: 'Session', hidden: false, pinned: false, last_used_at: null, started_at: 1, ended_at: null, branch: null, first_prompt: null, last_prompt: null, cwd: '/tmp/worktree', models: [], tool_count: 0, source_mtime: 1, partial: false };
}

describe('ProjectGroup identity and alias controls', () => {
  let app: ReturnType<typeof createApp> | undefined;
  let host: HTMLDivElement | undefined;
  afterEach(() => { app?.unmount(); host?.remove(); });

  it('shows stable, cwd, and worktree paths on hover and emits workspace alias reset from keyboard', async () => {
    const aliases: Array<[string, string | null]> = [];
    const project: Project = { id: 'legacy', workspace_id: 'workspace', name: 'Project', alias: 'Local name', path: '/repo', cwd_paths: ['/repo', '/repo/worktree'], worktree_paths: ['/repo/worktree'], latest_activity: 1, sessions: [session()] };
    host = document.createElement('div'); document.body.append(host);
    app = createApp(ProjectGroup, { project, selectedId: null, matches: new Map(), forceOpen: false, onAlias: (selection: [string, string | null]) => aliases.push(selection) });
    app.mount(host); await nextTick();
    expect(host.textContent).toContain('Local name');
    host.querySelector<HTMLDivElement>('.project-head-row')!.dispatchEvent(new Event('mouseenter')); await nextTick();
    expect(document.body.textContent).toContain('/repo/worktree');
    host.querySelector<HTMLButtonElement>('.alias-action')!.click(); await nextTick();
    const input = host.querySelector<HTMLInputElement>('.project-alias-input')!;
    input.value = ''; input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); await nextTick();
    expect(aliases).toEqual([['claude', 'workspace', null]]);
  });

  it('flattens mixed-agent sessions and keeps OpenCode session actions available', async () => {
    const claude = session('claude', 'claude-session');
    const opencode = { ...session('opencode', 'opencode-session'), title: 'OpenCode session', source_title: 'OpenCode session' };
    const project: Project = {
      id: 'legacy', workspace_id: 'workspace', name: 'Project', alias: null, path: '/repo',
      cwd_paths: [], worktree_paths: [], latest_activity: 1,
      sessions: [claude, opencode],
      agents: [
        { provider_id: 'claude', name: 'Claude', capabilities: { supports_reader: true, supports_search: true, supports_resume: true, supports_branching: true, supports_worktree: true, supports_changes: true }, sessions: [claude] },
        { provider_id: 'opencode', name: 'OpenCode', capabilities: { supports_reader: true, supports_search: true, supports_resume: false, supports_branching: false, supports_worktree: false, supports_changes: true }, sessions: [opencode] },
      ],
    };
    const pins: Array<[string, boolean]> = [];
    const hides: string[] = [];
    const renames: Array<[string, string | null]> = [];
    host = document.createElement('div'); document.body.append(host);
    app = createApp(ProjectGroup, {
      project,
      selectedId: null,
      matches: new Map(),
      forceOpen: false,
      onPin: (selection: [string, boolean]) => pins.push(selection),
      onHide: (id: string) => hides.push(id),
      onRename: (selection: [string, string | null]) => renames.push(selection),
    });
    app.mount(host); await nextTick();
    expect(host.querySelector('.alias-action')?.classList.contains('disabled')).toBe(true);
    host.querySelector<HTMLButtonElement>('.alias-action')!.click();
    await nextTick();
    expect(host.textContent).toContain('Filter to one agent');
    expect(host.querySelectorAll('.agent-group')).toHaveLength(0);
    expect(host.querySelectorAll('.session-item')).toHaveLength(2);
    const opencodeRow = [...host.querySelectorAll<HTMLElement>('.session-item-row')]
      .find((row) => row.textContent?.includes('OpenCode session'))!;
    expect(opencodeRow.querySelector('.agent-icon')).not.toBeNull();
    opencodeRow.querySelector<HTMLButtonElement>('[aria-label="Pin session"]')!.click();
    opencodeRow.querySelector<HTMLButtonElement>('.hide-action')!.click();
    opencodeRow.querySelector<HTMLButtonElement>('[aria-label="Rename session"]')!.click();
    await nextTick();
    const renameInput = opencodeRow.querySelector<HTMLInputElement>('.session-rename-input')!;
    renameInput.value = 'OpenCode renamed';
    renameInput.dispatchEvent(new Event('input', { bubbles: true }));
    opencodeRow.querySelector<HTMLFormElement>('.session-edit-form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    expect(pins).toEqual([['opencode-session', true]]);
    expect(hides).toEqual(['opencode-session']);
    expect(renames).toEqual([['opencode-session', 'OpenCode renamed']]);
  });
});
