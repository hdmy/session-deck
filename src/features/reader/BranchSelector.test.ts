import { createApp, nextTick } from 'vue';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { BranchSummary } from '../../types';
import BranchSelector from './BranchSelector.vue';

const roots: HTMLElement[] = [];
const branch = (id: string, overrides: Partial<BranchSummary> = {}): BranchSummary => ({
  id, session_id: 's', label: id, kind: id === 'main' ? 'main' : 'alternate', root_uuid: null,
  leaf_uuid: null, fork_point_uuid: null, is_active: id === 'main', event_count: 1, turn_count: 1,
  started_at: 1, ended_at: 2, compacted: false, ...overrides,
});

afterEach(() => { for (const root of roots.splice(0)) root.remove(); });

describe('BranchSelector', () => {
  it('keeps fork available for a single active branch while hiding only the list', async () => {
    const one = document.createElement('div');
    document.body.append(one); roots.push(one);
    createApp(BranchSelector, { branches: [branch('main')] }).mount(one);
    await nextTick();
    expect(one.querySelector('select')).toBeNull();
    expect(one.querySelector('.fork-button')?.hasAttribute('disabled')).toBe(false);

    const root = document.createElement('div');
    document.body.append(root); roots.push(root);
    const onSelect = vi.fn();
    createApp(BranchSelector, { branches: [branch('main'), branch('alt', { compacted: true })], onSelect, loading: true }).mount(root);
    await nextTick();
    expect(root.querySelector('select')?.getAttribute('aria-label')).toBe('Select history branch');
    expect(root.textContent).toContain('active');
    expect(root.textContent).toContain('compacted');
    expect(root.querySelector('[role="status"]')?.textContent).toContain('Loading');
    expect(root.querySelector('select')?.hasAttribute('disabled')).toBe(true);
  });

  it('explains why an alternate branch cannot fork', async () => {
    const root = document.createElement('div');
    document.body.append(root); roots.push(root);
    createApp(BranchSelector, {
      branches: [branch('main'), branch('alt')],
      forkDisabled: true,
      forkDisabledReason: '公共 Claude CLI 仅支持从当前主分支头分叉；alternate 分支仅支持只读历史浏览',
    }).mount(root);
    await nextTick();
    const button = root.querySelector<HTMLButtonElement>('.fork-button');
    expect(button?.disabled).toBe(true);
    expect(root.textContent).toContain('当前主分支头分叉');
    expect(button?.getAttribute('aria-describedby')).toBe('fork-disabled-reason');
  });
});
