import { createApp, nextTick } from 'vue';
import { afterEach, describe, expect, it } from 'vitest';
import type { FileChangeSummary } from '../../types';
import TurnChangeSummary from './TurnChangeSummary.vue';
import { normalizeFilePath } from './turnSummary';

const roots: HTMLElement[] = [];
const change = (overrides: Partial<FileChangeSummary> = {}): FileChangeSummary => ({ path: 'src/main.ts', kind: 'modified', additions: 2, deletions: 1, turn_id: 1, event_id: 2, tool_use_id: null, ...overrides });
afterEach(() => { for (const root of roots.splice(0)) root.remove(); });

describe('TurnChangeSummary', () => {
  it('normalizes paths and renders aggregate lines without raw content', async () => {
    const root = document.createElement('div'); document.body.append(root); roots.push(root);
    createApp(TurnChangeSummary, { changes: [change({ path: './src/../src/<unsafe>.ts' })] }).mount(root);
    await nextTick();
    expect(root.textContent).toContain('src/<unsafe>.ts');
    expect(root.textContent).toContain('+2');
    expect(root.textContent).toContain('−1');
    expect(root.textContent).not.toContain('assistant raw response');
  });

  it('does not render an empty change summary', async () => {
    const root = document.createElement('div'); document.body.append(root); roots.push(root);
    createApp(TurnChangeSummary, { changes: [] }).mount(root);
    await nextTick();
    expect(root.querySelector('[data-testid="turn-change-summary"]')).toBeNull();
  });

  it('drops traversal above the display root', () => {
    expect(normalizeFilePath('../../src/file.ts')).toBe('src/file.ts');
  });
});
