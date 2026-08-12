import { createApp, nextTick } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLocale } from '../../i18n';

const { mocks } = vi.hoisted(() => ({ mocks: {
  getClaudeSettings: vi.fn(), getScanSettings: vi.fn(), getIndexDiagnostics: vi.fn(),
  updateClaudeSettings: vi.fn(), updateScanSettings: vi.fn(), activateClaudeSourceRoot: vi.fn(), resumePreflight: vi.fn(),
} }));
vi.mock('../../api', () => ({ api: mocks }));

import SettingsPanel from './SettingsPanel.vue';

async function settle() { for (let i = 0; i < 8; i += 1) { await nextTick(); await Promise.resolve(); } }

const providers = [
  {
    provider_id: 'claude', name: 'Claude', capabilities: {
      supports_reader: true, supports_search: true, supports_resume: true,
      supports_branching: true, supports_worktree: true, supports_changes: true,
    },
  },
  {
    provider_id: 'codex', name: 'Codex', capabilities: {
      supports_reader: true, supports_search: true, supports_resume: false,
      supports_branching: false, supports_worktree: false, supports_changes: false,
    },
  },
  {
    provider_id: 'opencode', name: 'OpenCode', capabilities: {
      supports_reader: true, supports_search: true, supports_resume: false,
      supports_branching: false, supports_worktree: false, supports_changes: false,
    },
  },
] as const;

describe('SettingsPanel scan controls', () => {
  let host: HTMLDivElement;
  let app: ReturnType<typeof createApp>;
  beforeEach(() => {
    vi.clearAllMocks();
    setLocale('en');
    mocks.getClaudeSettings.mockResolvedValue({ executable_override: null, dangerously_skip_permissions: false });
    mocks.getScanSettings.mockResolvedValue({ source_root: '/old', effective_root: '/old', scan_interval_seconds: 60, enabled_provider_ids: ['claude'], provider_lookback_days: {} });
    mocks.getIndexDiagnostics.mockResolvedValue({ effective_root: '/old', scan_interval_seconds: 60, last_success_at: 1, last_attempt_at: 2, last_outcome: 'committed', indexed_sessions: 1, last_run: null, diagnostic_counts: [{ code: 'malformed_json', count: 2, last_occurred_at: 2, last_run_id: 1 }] });
    mocks.updateClaudeSettings.mockResolvedValue({ executable_override: null, dangerously_skip_permissions: false });
    mocks.updateScanSettings.mockImplementation(async (update) => ({ source_root: '/old', effective_root: '/old', ...update }));
    host = document.createElement('div'); document.body.append(host);
    app = createApp(SettingsPanel, { session: null, providers }); app.mount(host);
  });
  afterEach(() => { app.unmount(); host.remove(); setLocale('en'); });

  it('loads diagnostics and requires explicit root replacement confirmation', async () => {
    await settle();
    expect(host.textContent).toContain('malformed_json');
    const input = host.querySelector<HTMLInputElement>('.root-field input');
    expect(input).not.toBeNull();
    input!.value = '/new'; input!.dispatchEvent(new Event('input', { bubbles: true })); await nextTick();
    host.querySelector<HTMLButtonElement>('.root-section .secondary-button')!.click(); await settle();
    expect(mocks.activateClaudeSourceRoot).not.toHaveBeenCalled();
    expect(host.textContent).toContain('Confirm replacing the current local index first.');
  });

  it('keeps a draft when root activation fails', async () => {
    mocks.activateClaudeSourceRoot.mockRejectedValue(new Error('root unavailable'));
    await settle();
    const input = host.querySelector<HTMLInputElement>('.root-field input')!;
    input.value = '/new'; input.dispatchEvent(new Event('input', { bubbles: true }));
    host.querySelector<HTMLInputElement>('.confirm-row input')!.click();
    await nextTick();
    host.querySelector<HTMLButtonElement>('.root-section .secondary-button')!.click(); await settle();
    expect(host.textContent).toContain('root unavailable');
    expect(host.querySelector<HTMLInputElement>('.root-field input')?.value).toBe('/new');
  });

  it('keeps scan controls usable when Claude settings fail independently', async () => {
    app.unmount();
    mocks.getClaudeSettings.mockRejectedValue(new Error('claude settings unavailable'));
    app = createApp(SettingsPanel, { session: null, providers }); app.mount(host);
    await settle();
    expect(host.textContent).toContain('Claude settings unavailable');
    expect(host.querySelector<HTMLButtonElement>('.settings-footer .primary-button')?.disabled).toBe(true);
    expect(host.querySelector<HTMLButtonElement>('.scan-section .secondary-button')?.disabled).toBe(false);
  });

  it('keeps Claude controls usable when scan settings fail independently', async () => {
    app.unmount();
    mocks.getScanSettings.mockRejectedValue(new Error('scan settings unavailable'));
    app = createApp(SettingsPanel, { session: null, providers }); app.mount(host);
    await settle();
    expect(host.textContent).toContain('Scan settings unavailable');
    expect(host.querySelector<HTMLButtonElement>('.settings-footer .primary-button')?.disabled).toBe(false);
    expect(host.querySelector('.root-section')).toBeNull();
  });

  it('pulls an escaped Tab focus back into the dialog', async () => {
    await settle();
    const outside = document.createElement('button'); outside.textContent = 'outside'; document.body.append(outside); outside.focus();
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }));
    expect(host.querySelector('.settings-panel')?.contains(document.activeElement)).toBe(true);
    outside.remove();
  });

  it('renders Claude controls while scan settings remain deferred', async () => {
    app.unmount();
    let release!: (value: any) => void;
    mocks.getScanSettings.mockReturnValue(new Promise((resolve) => { release = resolve; }));
    app = createApp(SettingsPanel, { session: null, providers }); app.mount(host);
    await nextTick(); await Promise.resolve(); await nextTick();
    expect(host.querySelector('.settings-form')).not.toBeNull();
    expect(host.querySelector<HTMLInputElement>('input[placeholder="Automatic: claude"]')?.disabled).toBe(false);
    expect(host.textContent).toContain('Scan settings loading');
    release({ source_root: '/old', effective_root: '/old', scan_interval_seconds: 60, enabled_provider_ids: ['claude'], provider_lookback_days: {} });
  });

  it('imports only explicitly checked agents', async () => {
    await settle();
    host.querySelector<HTMLInputElement>('.agent-option input[value="codex"]')!.click();
    await nextTick();
    host.querySelector<HTMLButtonElement>('.scan-section .secondary-button')!.click();
    await settle();

    expect(mocks.updateScanSettings).toHaveBeenCalledWith({
      scan_interval_seconds: 60,
      enabled_provider_ids: ['claude', 'codex'],
      provider_lookback_days: {},
    });
  });

  it('saves a per-agent recent-days import range', async () => {
    await settle();
    host.querySelector<HTMLInputElement>('.agent-option input[value="codex"]')!.click();
    await nextTick();
    const range = host.querySelector<HTMLSelectElement>('.agent-lookback[data-provider-id="codex"]')!;
    range.value = '30';
    range.dispatchEvent(new Event('change', { bubbles: true }));
    host.querySelector<HTMLButtonElement>('.scan-section .secondary-button')!.click();
    await settle();

    expect(mocks.updateScanSettings).toHaveBeenCalledWith({
      scan_interval_seconds: 60,
      enabled_provider_ids: ['claude', 'codex'],
      provider_lookback_days: { codex: 30 },
    });
  });

  it('saves the provider selection from the main settings button and reloads it', async () => {
    let persistedProviders = ['claude'];
    mocks.getScanSettings.mockImplementation(async () => ({ source_root: '/old', effective_root: '/old', scan_interval_seconds: 60, enabled_provider_ids: [...persistedProviders], provider_lookback_days: {} }));
    mocks.updateScanSettings.mockImplementation(async (update) => {
      persistedProviders = [...update.enabled_provider_ids];
      return { source_root: '/old', effective_root: '/old', ...update };
    });

    await settle();
    host.querySelector<HTMLInputElement>('.agent-option input[value="opencode"]')!.click();
    host.querySelector<HTMLButtonElement>('.settings-footer .primary-button')!.click();
    await settle();

    expect(mocks.updateScanSettings).toHaveBeenCalledWith({
      scan_interval_seconds: 60,
      enabled_provider_ids: ['claude', 'opencode'],
      provider_lookback_days: {},
    });
    app.unmount();
    app = createApp(SettingsPanel, { session: null, providers }); app.mount(host);
    await settle();
    expect(host.querySelector<HTMLInputElement>('.agent-option input[value="opencode"]')?.checked).toBe(true);
  });

  it('shows provider save errors beside the footer actions', async () => {
    mocks.updateScanSettings.mockRejectedValue(new Error('lifecycle busy'));
    await settle();
    host.querySelector<HTMLInputElement>('.agent-option input[value="codex"]')!.click();
    host.querySelector<HTMLButtonElement>('.settings-footer .primary-button')!.click();
    await settle();

    const footer = host.querySelector<HTMLElement>('.settings-footer')!;
    expect(footer.querySelector('[role="alert"]')?.textContent).toContain('lifecycle busy');
    expect(footer.querySelectorAll('.settings-actions button')).toHaveLength(2);
  });

  it('reports committed partial root activation as updated but partial', async () => {
    mocks.activateClaudeSourceRoot.mockResolvedValue({
      settings: { source_root: '/new', effective_root: '/new', scan_interval_seconds: 60, enabled_provider_ids: ['claude'], provider_lookback_days: {} },
      scan: { root: '/new', trigger: 'manual', outcome: 'committed', committed: true, sessions: 1, diagnostics: 1, partial: true, removed_sessions: 0, new_files: 1, changed_files: 0, unchanged_files: 0, removed_files: 0, partial_sessions: 1 },
    });
    await settle();
    const input = host.querySelector<HTMLInputElement>('.root-field input')!;
    input.value = '/new'; input.dispatchEvent(new Event('input', { bubbles: true }));
    host.querySelector<HTMLInputElement>('.confirm-row input')!.click();
    await nextTick();
    host.querySelector<HTMLButtonElement>('.root-section .secondary-button')!.click(); await settle();
    expect(host.textContent).toContain('The root was committed, but the scan has partial diagnostics');
  });

  it('keeps the language draft until the main settings button saves it', async () => {
    await settle();
    const language = host.querySelector<HTMLSelectElement>('.language-field select');
    expect(language?.value).toBe('en');
    expect(host.textContent).toContain('Claude settings');
    language!.value = 'zh';
    language!.dispatchEvent(new Event('change', { bubbles: true }));
    await nextTick();
    expect(language?.value).toBe('zh');
    expect(host.textContent).toContain('Claude settings');
    expect(window.localStorage.getItem('context-vault.locale')).toBe('en');
    host.querySelector<HTMLButtonElement>('.settings-footer .primary-button')!.click();
    await settle();
    expect(host.textContent).toContain('Claude 设置');
    expect(window.localStorage.getItem('context-vault.locale')).toBe('zh');
  });
});
