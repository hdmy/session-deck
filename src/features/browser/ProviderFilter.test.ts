import { createApp, nextTick } from 'vue';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDescriptor } from '../../types';
import ProviderFilter from './ProviderFilter.vue';

const providers: ProviderDescriptor[] = [
  {
    provider_id: 'claude',
    name: 'Claude Code',
    capabilities: {
      supports_reader: true,
      supports_search: true,
      supports_resume: true,
      supports_branching: true,
      supports_worktree: true,
      supports_changes: true,
    },
  },
  {
    provider_id: 'codex',
    name: 'Codex',
    capabilities: {
      supports_reader: true,
      supports_search: true,
      supports_resume: false,
      supports_branching: false,
      supports_worktree: false,
      supports_changes: false,
    },
  },
];

describe('ProviderFilter', () => {
  let app: ReturnType<typeof createApp> | undefined;
  let host: HTMLDivElement | undefined;

  afterEach(() => {
    app?.unmount();
    host?.remove();
  });

  it('emits the selected provider while keeping an all-providers option', async () => {
    const onChange = vi.fn();
    host = document.createElement('div');
    document.body.append(host);
    app = createApp(ProviderFilter, { providers, onChange });
    app.mount(host);

    const select = host.querySelector<HTMLSelectElement>('select')!;
    select.value = 'codex';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await nextTick();

    expect(onChange).toHaveBeenCalledWith('codex');
    expect(host.textContent).toContain('Codex');
  });
});
