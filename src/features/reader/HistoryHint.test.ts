import { createApp, nextTick } from 'vue';
import { afterEach, describe, expect, it } from 'vitest';
import type { BranchSummary, SessionDetail, SessionSummary } from '../../types';
import HistoryHint from './HistoryHint.vue';

const roots: HTMLElement[] = [];
const summary: SessionSummary = {
  id: 's', native_session_id: 'n', provider_id: 'claude', project_id: 'p', title: 'Session', source_title: 'Session',
  hidden: false, pinned: false, last_used_at: null, started_at: 1, ended_at: 2, branch: 'main', first_prompt: 'hello', last_prompt: 'hello',
  cwd: '/tmp/project', models: [], tool_count: 0, source_mtime: 1, partial: false,
};
const branch = (overrides: Partial<BranchSummary>): BranchSummary => ({
  id: 'main', session_id: 's', label: 'main', kind: 'main', root_uuid: null, leaf_uuid: null, fork_point_uuid: null,
  is_active: true, event_count: 4, turn_count: 2, started_at: 1, ended_at: 2, compacted: false, ...overrides,
});

afterEach(() => { for (const root of roots.splice(0)) root.remove(); });

function mount(detail: SessionDetail): HTMLElement {
  const root = document.createElement('div');
  document.body.append(root); roots.push(root);
  createApp(HistoryHint, { detail }).mount(root);
  return root;
}

describe('HistoryHint', () => {
  it('describes alternate recovery view, fork point, and branch range', async () => {
    const root = mount({
      summary, timeline: [], diagnostics: [], selected_branch_id: 'alt', active_branch_id: 'main',
      branches: [branch({}), branch({ id: 'alt', label: 'alternate-1', kind: 'alternate', is_active: false, fork_point_uuid: '12345678-abcdef', event_count: 7, turn_count: 3 })],
    });
    await nextTick();
    expect(root.textContent).toContain('read-only history view');
    expect(root.textContent).toContain('12345678…cdef');
    expect(root.textContent).toContain('7 events and 3 turns');
  });

  it('shows compact impact and alternate count on the main branch', async () => {
    const root = mount({
      summary, timeline: [{ id: 1, session_id: 's', kind: 'system', role: null, content: '', timestamp: null, tool_name: null, collapsed: true, compact_boundary: true }], diagnostics: [], selected_branch_id: 'main', active_branch_id: 'main',
      branches: [branch({}), branch({ id: 'alt', kind: 'alternate', is_active: false })],
    });
    await nextTick();
    expect(root.textContent).toContain('1 other historical branches are available');
    expect(root.textContent).toContain('may have been compressed');
  });

  it('includes relation provider, type, and status as escaped text', async () => {
    const root = mount({
      summary, timeline: [], diagnostics: [],
      relations: [{ provider_id: 'claude', parent_session_id: 's', child_session_id: 'child', relation_type: 'fork', created_at: 1, status: 'pending', parent_present: true, child_present: false }],
    });
    await nextTick();
    expect(root.textContent).toContain('provider claude');
    expect(root.textContent).toContain('relation fork');
    expect(root.textContent).toContain('status pending');
    expect(root.querySelector('script')).toBeNull();
  });

  it('shows cwd history only when cwd changes and keeps it keyboard-foldable', async () => {
    const root = mount({
      summary, timeline: [], diagnostics: [],
      cwd_history: [
        { cwd: '/tmp/project', first_sequence: 1, last_sequence: 2, first_timestamp: 1, last_timestamp: 2, resume: false },
        { cwd: '/tmp/project/worktree', first_sequence: 3, last_sequence: 4, first_timestamp: 3, last_timestamp: 4, resume: true },
      ],
    });
    await nextTick();
    const toggle = root.querySelector<HTMLElement>('.cwd-history summary');
    expect(toggle?.textContent).toContain('Historical cwd changes (2)');
    expect(toggle?.tabIndex).toBeGreaterThanOrEqual(0);
    expect(root.textContent).toContain('Follow-up cwd');
    const single = mount({ summary, timeline: [], diagnostics: [], cwd_history: [{ cwd: '/tmp/project', first_sequence: 1, last_sequence: 1, first_timestamp: 1, last_timestamp: 1, resume: false }] });
    await nextTick();
    expect(single.querySelector('.cwd-history')).toBeNull();
  });
});
