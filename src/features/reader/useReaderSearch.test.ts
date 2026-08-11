import { effectScope, shallowRef } from 'vue';
import { afterEach, describe, expect, it } from 'vitest';
import type { SessionDetail, SessionSummary } from '../../types';
import { useReaderSearch } from './useReaderSearch';

const summary: SessionSummary = {
  id: 'session', native_session_id: 'native', provider_id: 'claude', project_id: 'project', title: 'Session', source_title: 'Session',
  hidden: false, pinned: false, last_used_at: null, started_at: 1, ended_at: 2, branch: 'main', first_prompt: 'needle', last_prompt: 'needle',
  cwd: '/tmp/project', models: [], tool_count: 0, source_mtime: 1, partial: false,
};

function detail(content = 'needle'): SessionDetail {
  return { summary, timeline: [{ id: 7, session_id: 'session', kind: 'user', role: 'user', content, timestamp: 1, tool_name: null, collapsed: false }], diagnostics: [] };
}

describe('useReaderSearch', () => {
  let scope: ReturnType<typeof effectScope> | null = null;
  afterEach(() => { scope?.stop(); scope = null; });

  it('starts a new query before the first match and locates it on Next', () => {
    const source = shallowRef<SessionDetail | null>(detail());
    const mode = shallowRef<'focus' | 'full'>('focus');
    let search!: ReturnType<typeof useReaderSearch>;
    scope = effectScope();
    scope.run(() => { search = useReaderSearch({ detail: source, mode }); });
    search.setQuery('needle');
    expect(search.currentIndex.value).toBe(-1);
    expect(search.matchCount.value).toBe(1);
    search.next();
    expect(search.currentIndex.value).toBe(0);
  });
});
