import { createApp, nextTick } from 'vue';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { BranchSummary, KnowledgeCard, SessionDetail, SessionSummary } from '../../types';
import ContextReader from './ContextReader.vue';

vi.mock('../../api', () => ({ api: {} }));

const roots: HTMLElement[] = [];

function session(id: string, title: string): SessionSummary {
  return {
    id, native_session_id: `native-${id}`, provider_id: 'claude', project_id: 'project-1', title,
    source_title: title, hidden: false, pinned: false, last_used_at: null, started_at: 1,
    ended_at: null, branch: 'main', first_prompt: 'hello', last_prompt: 'hello', cwd: '/tmp/project',
    models: ['claude'], tool_count: 0, source_mtime: 1, partial: false,
  };
}

function branch(id: string, active: boolean): BranchSummary {
  return { id, session_id: 'session-1', label: id, kind: active ? 'active' : 'alternate', root_uuid: null, leaf_uuid: null, fork_point_uuid: null, is_active: active, event_count: 1, turn_count: 1, started_at: 1, ended_at: 2, compacted: false };
}

const claudeCapabilities = {
  supports_reader: true, supports_search: true, supports_resume: true,
  supports_branching: true, supports_worktree: true, supports_changes: true,
};

function mount(props: Record<string, unknown>) {
  const root = document.createElement('div');
  document.body.append(root);
  roots.push(root);
  const onContinue = vi.fn();
  createApp(ContextReader, { ...props, onContinue }).mount(root);
  return { root, onContinue };
}

afterEach(() => {
  for (const root of roots.splice(0)) root.remove();
});

describe('ContextReader continuation state', () => {
  it('allows fork only for an explicitly selected active head', async () => {
    const detail: SessionDetail = {
      summary: session('session-1', 'Forkable'), timeline: [], diagnostics: [],
      branches: [branch('main', true)], active_branch_id: 'main', selected_branch_id: 'main',
    };
    const { root } = mount({ detail, loading: false, error: null, providerCapabilities: claudeCapabilities });
    await nextTick();
    expect(root.querySelector<HTMLButtonElement>('.fork-button')?.disabled).toBe(false);

    const alternate: SessionDetail = { ...detail, branches: [branch('main', true), branch('alt', false)], selected_branch_id: 'alt' };
    const second = mount({ detail: alternate, loading: false, error: null });
    await nextTick();
    expect(second.root.querySelector<HTMLButtonElement>('.fork-button')?.disabled).toBe(true);
    expect(second.root.textContent).toContain('Fork from current branch head');
  });

  it('keeps continuation disabled when provider capabilities are missing', async () => {
    const detail: SessionDetail = {
      summary: session('session-unknown', 'Unknown provider'), timeline: [], diagnostics: [],
      branches: [branch('main', true)], active_branch_id: 'main', selected_branch_id: 'main',
    };
    const { root } = mount({ detail, loading: false, error: null });
    await nextTick();
    expect(root.querySelector<HTMLButtonElement>('.continue-button')?.disabled).toBe(true);
    expect(root.querySelector<HTMLButtonElement>('.fork-button')?.disabled).toBe(true);
    expect(root.textContent).toContain('This agent does not support continuation');
  });

  it('disables Continue with a reason while another session is active', async () => {
    const detail: SessionDetail = { summary: session('session-2', 'Other session'), timeline: [], diagnostics: [] };
    const { root, onContinue } = mount({
      detail, loading: false, error: null, activeContinuationSessionId: 'session-1', activeContinuationTitle: 'Active session',
      continuationPhase: 'running', liveEvents: [], tailPartial: false, tailDiagnostics: 0, tailError: null,
    });
    await nextTick();
    const button = root.querySelector<HTMLButtonElement>('.continue-button');
    expect(button?.disabled).toBe(true);
    expect(root.textContent).toContain('Active session');
    expect(root.textContent).toContain('Running');
    button?.click();
    expect(onContinue).not.toHaveBeenCalled();
  });

  it('disables Continue while branch data is loading or an alternate is selected', async () => {
    const detail: SessionDetail = {
      summary: session('session-5', 'Branching'), timeline: [], diagnostics: [],
      branches: [branch('main', true)], active_branch_id: 'main', selected_branch_id: 'main',
    };
    const loading = mount({ detail, loading: false, error: null, branchLoading: true });
    await nextTick();
    expect(loading.root.querySelector<HTMLButtonElement>('.continue-button')?.disabled).toBe(true);
    expect(loading.root.textContent).toContain('Loading branch');

    const alternate: SessionDetail = { ...detail, branches: [branch('main', true), branch('alt', false)], selected_branch_id: 'alt' };
    const selectedAlternate = mount({ detail: alternate, loading: false, error: null });
    await nextTick();
    expect(selectedAlternate.root.querySelector<HTMLButtonElement>('.continue-button')?.disabled).toBe(true);
    expect(selectedAlternate.root.textContent).toContain('alternate');
  });

  it('keeps the current session title and phase visible during continuation', async () => {
    const detail: SessionDetail = { summary: session('session-1', 'Current session'), timeline: [], diagnostics: [] };
    const { root } = mount({
      detail, loading: false, error: null, activeContinuationSessionId: 'session-1', continuationPhase: 'draining',
      liveEvents: [], tailPartial: true, tailDiagnostics: 1, tailError: null,
    });
    await nextTick();
    expect(root.textContent).toContain('Current session');
    expect(root.textContent).toContain('Draining');
    expect(root.querySelector('.live-transcript')).not.toBeNull();
  });

  it('renders turn file changes and tool summaries from normalized detail only', async () => {
    const detail: SessionDetail = {
      summary: session('session-3', 'Insights'),
      timeline: [
        { id: 1, session_id: 'session-3', kind: 'user', role: 'user', content: 'prompt', timestamp: 1, tool_name: null, collapsed: false },
        { id: 2, session_id: 'session-3', kind: 'assistant', role: 'assistant', content: 'final', timestamp: 2, tool_name: null, collapsed: false },
      ],
      diagnostics: [],
      tool_stats: [{ name: 'Edit', count: 1, successes: 1, failures: 0, unknown: 0, files_changed: 1, additions: 3, deletions: 1 }],
      turn_insights: [{ turn_id: 1, file_changes: [{ path: 'src/app.ts', kind: 'modified', additions: 3, deletions: 1, turn_id: 1, event_id: 2, tool_use_id: null }], tool_stats: [{ name: 'Edit', count: 1, successes: 1, failures: 0, unknown: 0, files_changed: 1, additions: 3, deletions: 1 }] }],
    };
    const { root } = mount({ detail, loading: false, error: null });
    await nextTick();
    expect(root.querySelector('[data-testid="turn-change-summary"]')).not.toBeNull();
    expect(root.querySelector('[data-testid="tool-summary"]')).not.toBeNull();
    expect(root.textContent).toContain('src/app.ts');
    expect(root.textContent).not.toContain('raw tool output');
  });

  it('renders backend turns and alternate relation hints without raw tool payloads', async () => {
    const detail: SessionDetail = {
      summary: session('session-4', 'Backend insights'),
      timeline: [],
      turns: [{
        id: 1,
        session_id: 'session-4',
        user_prompt: 'make the change',
        timestamp: 1,
        completed: true,
        final_response: 'done',
        activities: [{
          event_id: 2, kind: 'assistant', role: 'assistant', content: 'done', timestamp: 2,
          tool_name: null, tool_use_id: null, parent_tool_use_id: null, collapsed: false, final_response: true,
        }],
      }],
      diagnostics: [],
      branches: [branch('main', true), { ...branch('alt', false), fork_point_uuid: 'fork-point' }],
      active_branch_id: 'main',
      selected_branch_id: 'alt',
      turn_insights: [{ turn_id: 1, file_changes: [{ path: 'src/feature.ts', kind: 'modified', additions: 1, deletions: 0, turn_id: 1, event_id: 2, tool_use_id: null }], tool_stats: [] }],
      relations: [{ provider_id: 'claude', parent_session_id: 'session-4', child_session_id: 'child', relation_type: 'fork', created_at: 2, status: 'pending', parent_present: true, child_present: false }],
    };
    const { root } = mount({ detail, loading: false, error: null });
    await nextTick();
    expect(root.textContent).toContain('make the change');
    expect(root.textContent).toContain('src/feature.ts');
    expect(root.textContent).toContain('read-only history view');
    expect(root.textContent).toContain('Child session');
    expect(root.textContent).not.toContain('raw tool output');
  });

  it('keeps duplicate knowledge metadata out of the primary reader', async () => {
    const detail: SessionDetail = { summary: session('session-knowledge', 'Same title'), timeline: [], diagnostics: [] };
    const card: KnowledgeCard = {
      session_id: 'session-knowledge',
      title: 'Same title',
      summary: 'Same title',
      topics: [],
      tags: [],
      decisions: [],
      troubleshooting: [],
      change_summary: '',
      body_markdown: '',
      source_session_ids: ['session-knowledge'],
      auto_generated: true,
      updated_at: null,
    };
    const { root } = mount({ detail, loading: false, error: null, knowledgeCard: card, knowledgeLoading: false, knowledgeError: null });
    await nextTick();
    expect(root.querySelector<HTMLDetailsElement>('.knowledge-reader-section')?.open).toBe(false);
    expect(root.querySelector('.knowledge-card-panel')).toBeNull();
  });
});
