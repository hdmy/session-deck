import { createApp, nextTick } from 'vue';
import { afterEach, describe, expect, it } from 'vitest';
import type { ToolStat } from '../../types';
import ToolSummary from './ToolSummary.vue';

const roots: HTMLElement[] = [];
const stat = (overrides: Partial<ToolStat> = {}): ToolStat => ({ name: 'Edit', count: 3, successes: 2, failures: 1, unknown: 0, files_changed: 2, additions: 8, deletions: 3, ...overrides });
afterEach(() => { for (const root of roots.splice(0)) root.remove(); });

describe('ToolSummary', () => {
  it('groups totals by tool and starts collapsed but keyboard reachable', async () => {
    const root = document.createElement('div'); document.body.append(root); roots.push(root);
    createApp(ToolSummary, { stats: [stat(), stat({ name: 'Bash', count: 1, successes: 0, failures: 0, unknown: 1, files_changed: 0, additions: 0, deletions: 0 })] }).mount(root);
    await nextTick();
    const details = root.querySelector('details');
    expect(details?.open).toBe(false);
    expect(root.textContent).toContain('4 total');
    expect(root.textContent).toContain('2 success');
    expect(root.querySelector('summary')?.getAttribute('tabindex')).not.toBe('-1');
  });
});
