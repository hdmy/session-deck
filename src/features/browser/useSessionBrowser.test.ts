import { effectScope, nextTick } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BranchSummary, Project, ScanReport, SearchHit, SessionDetail, SessionSummary } from '../../types';

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    scan: vi.fn(),
    projects: vi.fn(),
    session: vi.fn(),
    getSessionBranch: vi.fn(),
    search: vi.fn(),
    listProviderDescriptors: vi.fn(),
    listHiddenSessions: vi.fn(),
    setSessionHidden: vi.fn(),
    renameSession: vi.fn(),
    setSessionPinned: vi.fn(),
    setProjectAlias: vi.fn(),
    activateClaudeSourceRoot: vi.fn(),
    touchSession: vi.fn(),
  },
}));

vi.mock('../../api', () => ({ api: apiMock }));

import { useSessionBrowser } from './useSessionBrowser';

function makeSession(id = 'session-1', overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    id,
    provider_id: 'claude',
    project_id: 'project-1',
    title: id,
    source_title: id,
    hidden: false,
    pinned: false,
    last_used_at: null,
    started_at: 10,
    ended_at: null,
    branch: null,
    first_prompt: null,
    last_prompt: null,
    cwd: '/tmp/project-1',
    models: [],
    tool_count: 0,
    source_mtime: 1,
    partial: false,
    ...overrides,
  };
}

function makeProject(sessions: SessionSummary[] = [makeSession()]): Project {
  return {
    id: 'project-1',
    name: 'project-1',
    path: '/tmp/project-1',
    latest_activity: 10,
    sessions,
  };
}

function makeDetail(summary: SessionSummary): SessionDetail {
  return { summary, timeline: [], diagnostics: [] };
}

function makeBranch(id: string, overrides: Partial<BranchSummary> = {}): BranchSummary {
  return {
    id, session_id: 'session-1', label: id, kind: id === 'main' ? 'main' : 'alternate',
    root_uuid: null, leaf_uuid: null, fork_point_uuid: null, is_active: id === 'main',
    event_count: 1, turn_count: 1, started_at: 1, ended_at: 2, compacted: false, ...overrides,
  };
}

function makeReport(overrides: Partial<ScanReport> = {}): ScanReport {
  return { root: '/tmp', trigger: 'manual', outcome: 'committed', committed: true, sessions: 1, diagnostics: 0, partial: false, removed_sessions: 0, new_files: 0, changed_files: 0, unchanged_files: 0, removed_files: 0, partial_sessions: 0, ...overrides };
}

function makeHit(session: SessionSummary, event_id = 1): SearchHit {
  return { session, snippet: `snippet:${session.title}`, event_id };
}

async function settle(): Promise<void> {
  await nextTick();
  await Promise.resolve();
  await Promise.resolve();
}

describe('useSessionBrowser', () => {
  let scope: ReturnType<typeof effectScope>;
  let browser: ReturnType<typeof useSessionBrowser>;

  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    const session = makeSession();
    apiMock.scan.mockResolvedValue(makeReport());
    apiMock.projects.mockResolvedValue([makeProject([session])]);
    apiMock.listHiddenSessions.mockResolvedValue([]);
    apiMock.session.mockResolvedValue(makeDetail(session));
    apiMock.getSessionBranch.mockResolvedValue(makeDetail(session));
    apiMock.search.mockResolvedValue([makeHit(session)]);
    apiMock.listProviderDescriptors.mockResolvedValue([]);
    apiMock.touchSession.mockResolvedValue({ ...session, last_used_at: 100 });
    apiMock.setSessionHidden.mockImplementation(async (_id: string, hidden: boolean) => ({ ...session, hidden }));
    apiMock.renameSession.mockImplementation(async (_id: string, title: string | null) => ({
      ...session,
      title: title ?? session.source_title,
    }));
    apiMock.setSessionPinned.mockImplementation(async (_id: string, pinned: boolean) => ({ ...session, pinned }));
    apiMock.setProjectAlias.mockResolvedValue([makeProject([session])]);
    apiMock.activateClaudeSourceRoot.mockResolvedValue({ settings: { source_root: '/tmp', effective_root: '/tmp', scan_interval_seconds: 0, enabled_provider_ids: ['claude'], provider_lookback_days: {} }, scan: makeReport() });

    scope = effectScope();
    scope.run(() => {
      browser = useSessionBrowser();
    });
  });

  afterEach(() => {
    scope.stop();
    vi.useRealTimers();
  });

  it('clears the current selection when hiding it', async () => {
    await browser.select('session-1');
    await browser.hide('session-1');

    expect(browser.selectedId.value).toBeNull();
    expect(browser.selected.value).toBeNull();
  });

  it('keeps hidden sessions in a loading state for the full scan', async () => {
    let releaseScan!: (report: ScanReport) => void;
    apiMock.scan.mockReturnValueOnce(new Promise<ScanReport>((resolve) => { releaseScan = resolve; }));

    const pending = browser.refresh();
    await settle();
    expect(browser.hiddenLoading.value).toBe(true);

    releaseScan(makeReport());
    await pending;
    expect(browser.hiddenLoading.value).toBe(false);
  });

  it('returns an explicit scan failure without treating it as source removal', async () => {
    apiMock.scan.mockRejectedValueOnce(new Error('scan unavailable'));
    const result = await browser.refresh();
    expect(result).toEqual({ status: 'error', error: 'scan unavailable' });
    expect(browser.sourceRemovalNotice.value).toBeNull();
  });

  it('returns a success report for a completed scan', async () => {
    const result = await browser.refresh();
    expect(result.status).toBe('success');
    if (result.status === 'success') expect(result.report.sessions).toBe(1);
  });

  it('loads provider descriptors and sends the selected provider in search payloads', async () => {
    apiMock.listProviderDescriptors.mockResolvedValueOnce([{
      provider_id: 'codex', name: 'Codex', capabilities: {
        supports_reader: true, supports_search: true, supports_resume: false,
        supports_branching: false, supports_worktree: false, supports_changes: false,
      },
    }]);
    await browser.refresh();
    expect(browser.providerDescriptors.value[0]?.provider_id).toBe('codex');

    browser.search('needle');
    await vi.advanceTimersByTimeAsync(180);
    browser.setProviderFilter('codex');
    await vi.advanceTimersByTimeAsync(180);
    expect(apiMock.search).toHaveBeenLastCalledWith('needle', 'codex');
  });

  it('always sends provider-scoped project alias payloads', async () => {
    await browser.refresh();
    await browser.setProjectAlias('codex', 'workspace-1', 'Codex project');
    expect(apiMock.setProjectAlias).toHaveBeenLastCalledWith('codex', 'workspace-1', 'Codex project');
  });

  it('restores and renames sessions, refreshing active search hits', async () => {
    const session = makeSession();
    let searchSession = session;
    apiMock.search.mockImplementation(async () => [makeHit(searchSession)]);
    apiMock.listHiddenSessions.mockResolvedValue([makeSession('hidden-1', { hidden: true })]);
    await browser.restore('hidden-1');
    expect(apiMock.setSessionHidden).toHaveBeenCalledWith('hidden-1', false);

    browser.search('needle');
    await vi.advanceTimersByTimeAsync(180);
    await settle();
    searchSession = { ...session, title: 'Renamed' };
    apiMock.renameSession.mockResolvedValue(searchSession);
    await browser.rename(session.id, 'Renamed');

    expect(apiMock.search).toHaveBeenCalledWith('needle');
    expect(browser.hits.value[0]?.session.title).toBe('Renamed');
  });

  it('keeps sorting state fresh after pin and touch mutations', async () => {
    const old = makeSession('old', { last_used_at: 1 });
    const recent = makeSession('recent', { last_used_at: 2 });
    apiMock.projects.mockResolvedValue([makeProject([old, recent])]);
    await browser.refresh();
    expect(browser.projects.value[0]?.sessions.map((session) => session.id)).toEqual(['recent', 'old']);

    apiMock.setSessionPinned.mockResolvedValue({ ...old, pinned: true });
    apiMock.projects.mockResolvedValue([makeProject([{ ...old, pinned: true }, recent])]);
    await browser.pin('old', true);
    expect(browser.projects.value[0]?.sessions[0]?.id).toBe('old');

    apiMock.session.mockResolvedValue(makeDetail(recent));
    await browser.select('recent');
    expect(apiMock.touchSession).not.toHaveBeenCalled();
    expect(browser.projects.value[0]?.sessions[1]?.id).toBe('recent');
  });

  it('keeps a session in its workspace project after local mutation', async () => {
    const session = makeSession('workspace-session', { project_id: 'legacy-project', workspace_id: 'workspace-1' });
    apiMock.projects.mockResolvedValue([{
      ...makeProject([session]), id: 'legacy-project', workspace_id: 'workspace-1',
    }]);
    apiMock.setSessionPinned.mockResolvedValue({ ...session, pinned: true });
    await browser.refresh();
    await browser.pin(session.id, true);
    expect(browser.projects.value[0]?.sessions.map((item) => item.id)).toEqual([session.id]);
  });

  it('keeps a cross-Agent session in its path-grouped project during local mutation', async () => {
    const claude = makeSession('claude-session', { workspace_id: '/repo/.git', project_path: '/repo' });
    const opencode = makeSession('opencode-session', {
      provider_id: 'opencode', workspace_id: 'opencode:dir:/repo', project_path: '/repo',
    });
    const project = { ...makeProject([claude, opencode]), id: '/repo', workspace_id: '/repo/.git', path: '/repo' };
    apiMock.projects.mockResolvedValueOnce([project]);
    await browser.refresh();

    apiMock.setSessionPinned.mockResolvedValue({ ...opencode, pinned: true });
    let release!: (projects: Project[]) => void;
    apiMock.projects.mockReturnValueOnce(new Promise<Project[]>((resolve) => { release = resolve; }));
    const pending = browser.pin(opencode.id, true);
    await settle();

    expect(browser.projects.value[0]?.sessions.map((item) => item.id)).toContain(opencode.id);
    release([{ ...project, sessions: [claude, { ...opencode, pinned: true }] }]);
    await pending;
  });

  it('ignores a late alias response after a newer navigation generation', async () => {
    await browser.refresh();
    let release!: (projects: Project[]) => void;
    apiMock.setProjectAlias.mockReturnValueOnce(new Promise<Project[]>((resolve) => { release = resolve; }));
    const pending = browser.setProjectAlias('project-1', 'old');
    apiMock.projects.mockResolvedValueOnce([makeProject([makeSession('new', { title: 'New' })])]);
    await browser.refresh();
    release([makeProject([makeSession('old', { title: 'Old' })])]);
    await pending;
    expect(browser.projects.value[0]?.sessions[0]?.title).toBe('New');
  });

  it('serializes a committed scheduled hydration behind the active scan owner', async () => {
    await browser.refresh();
    let releaseA!: (report: ScanReport) => void;
    apiMock.scan.mockReturnValueOnce(new Promise<ScanReport>((resolve) => { releaseA = resolve; }));
    const scanA = browser.refresh('manual');
    await settle();
    apiMock.scan.mockResolvedValueOnce(makeReport({ trigger: 'scheduled', outcome: 'committed', committed: true, new_files: 1 }));
    apiMock.projects.mockResolvedValue([makeProject([makeSession('newest', { title: 'Newest' })])]);
    const scanB = browser.refresh('scheduled');
    releaseA(makeReport({ trigger: 'manual', outcome: 'committed', committed: true, removed_sessions: 9 }));
    expect((await scanA).status).toBe('success');
    await scanB;
    expect(browser.projects.value[0]?.sessions[0]?.title).toBe('Newest');
    expect(browser.sourceRemovalNotice.value).toBeNull();
  });

  it('keeps the original refresh owner when a busy scheduled scan is skipped', async () => {
    await browser.refresh();
    let releaseA!: (report: ScanReport) => void;
    apiMock.scan.mockReturnValueOnce(new Promise<ScanReport>((resolve) => { releaseA = resolve; }));
    const scanA = browser.refresh('manual');
    await settle();
    apiMock.scan.mockResolvedValueOnce(makeReport({ trigger: 'scheduled', outcome: 'skipped_lifecycle', committed: false }));
    const scanB = browser.refresh('scheduled');
    expect((await scanB).status).toBe('skipped');
    releaseA(makeReport());
    expect((await scanA).status).toBe('success');
    expect(browser.scanning.value).toBe(false);
  });

  it('keeps the original refresh owner on a manual double-click', async () => {
    await browser.refresh();
    let release!: (report: ScanReport) => void;
    apiMock.scan.mockReturnValueOnce(new Promise<ScanReport>((resolve) => { release = resolve; }));
    const first = browser.refresh('manual');
    await settle();
    expect((await browser.refresh('manual')).status).toBe('skipped');
    release(makeReport());
    expect((await first).status).toBe('success');
    expect(browser.scanning.value).toBe(false);
  });

  it('keeps A ownership when B scheduled returns partial during A hydration', async () => {
    await browser.refresh();
    let releaseProjects!: (projects: Project[]) => void;
    apiMock.scan.mockResolvedValueOnce(makeReport({ committed: true, outcome: 'committed' }));
    apiMock.projects.mockReturnValueOnce(new Promise<Project[]>((resolve) => { releaseProjects = resolve; }));
    const scanA = browser.refresh('manual');
    await settle();
    apiMock.scan.mockResolvedValueOnce(makeReport({ trigger: 'scheduled', committed: false, outcome: 'partial', partial: true }));
    const scanB = browser.refresh('scheduled');
    const resultB = await scanB;
    expect(resultB.status).toBe('success');
    releaseProjects([makeProject([makeSession('a-new', { title: 'A committed' })])]);
    expect((await scanA).status).toBe('success');
    expect(browser.projects.value[0]?.sessions[0]?.title).toBe('A committed');
    expect(browser.partial.value).toBe(true);
    expect(browser.scanning.value).toBe(false);
  });

  it('serializes root activation after a pre-existing scan so the activation report wins', async () => {
    await browser.refresh();
    let releaseScan!: (report: ScanReport) => void;
    apiMock.scan.mockReturnValueOnce(new Promise<ScanReport>((resolve) => { releaseScan = resolve; }));
    const scan = browser.refresh('manual');
    await settle();
    const activation = browser.activateSourceRoot('/new-root', true);
    expect(apiMock.activateClaudeSourceRoot).not.toHaveBeenCalled();
    apiMock.activateClaudeSourceRoot.mockResolvedValueOnce({
      settings: { source_root: '/new-root', effective_root: '/new-root', scan_interval_seconds: 0, enabled_provider_ids: ['claude'], provider_lookback_days: {} },
      scan: makeReport({ root: '/new-root', partial: true, partial_sessions: 1 }),
    });
    releaseScan(makeReport({ new_files: 1 }));
    await scan;
    await activation;
    expect(apiMock.activateClaudeSourceRoot).toHaveBeenCalledWith('/new-root', true);
    expect(browser.lastScanReport.value?.root).toBe('/new-root');
    expect(browser.scanNotice.value).toContain('partial');
    expect(browser.scanning.value).toBe(false);
  });

  it('keeps later scans behind an in-flight root activation', async () => {
    await browser.refresh();
    let releaseActivation!: (value: { settings: { source_root: string; effective_root: string; scan_interval_seconds: number; enabled_provider_ids: ['claude']; provider_lookback_days: {} }; scan: ScanReport }) => void;
    apiMock.activateClaudeSourceRoot.mockReturnValueOnce(new Promise((resolve) => { releaseActivation = resolve; }));
    const activation = browser.activateSourceRoot('/new-root', true);
    await settle();
    apiMock.scan.mockResolvedValueOnce(makeReport({ trigger: 'scheduled', new_files: 1 }));
    const scheduled = browser.refresh('scheduled');
    await settle();
    expect(apiMock.scan).toHaveBeenCalledTimes(1);
    releaseActivation({
      settings: { source_root: '/new-root', effective_root: '/new-root', scan_interval_seconds: 0, enabled_provider_ids: ['claude'], provider_lookback_days: {} },
      scan: makeReport({ root: '/new-root', partial: true, partial_sessions: 1 }),
    });
    await activation;
    expect((await scheduled).status).toBe('success');
    expect(browser.lastScanReport.value?.root).toBe('/tmp');
    expect(browser.scanNotice.value).toContain('new 1');
    expect(browser.scanning.value).toBe(false);
  });

  it('lets a scheduled commit created after activation return win', async () => {
    await browser.refresh();
    let releaseActivation!: (value: { settings: { source_root: string; effective_root: string; scan_interval_seconds: number; enabled_provider_ids: ['claude']; provider_lookback_days: {} }; scan: ScanReport }) => void;
    apiMock.activateClaudeSourceRoot.mockReturnValueOnce(new Promise((resolve) => { releaseActivation = resolve; }));
    const activation = browser.activateSourceRoot('/new-root', true);
    await settle();
    apiMock.scan.mockResolvedValueOnce(makeReport({ trigger: 'scheduled', new_files: 9 }));
    const scheduled = browser.refresh('scheduled');
    await settle();
    expect(apiMock.scan).toHaveBeenCalledTimes(1);
    apiMock.projects.mockResolvedValue([makeProject([makeSession('scheduled', { title: 'Scheduled' })])]);
    releaseActivation({ settings: { source_root: '/new-root', effective_root: '/new-root', scan_interval_seconds: 0, enabled_provider_ids: ['claude'], provider_lookback_days: {} }, scan: makeReport({ root: '/new-root', partial: true, partial_sessions: 1 }) });
    await activation;
    expect((await scheduled).status).toBe('success');
    expect(browser.lastScanReport.value?.trigger).toBe('scheduled');
    expect(browser.projects.value[0]?.sessions[0]?.title).toBe('Scheduled');
  });

  it('reserves an idle activation lease before a near-synchronous refresh', async () => {
    await browser.refresh();
    let releaseActivation!: (value: { settings: { source_root: string; effective_root: string; scan_interval_seconds: number; enabled_provider_ids: ['claude']; provider_lookback_days: {} }; scan: ScanReport }) => void;
    apiMock.activateClaudeSourceRoot.mockReturnValueOnce(new Promise((resolve) => { releaseActivation = resolve; }));
    const activation = browser.activateSourceRoot('/new-root', true);
    const refresh = await browser.refresh('manual');
    expect(refresh).toEqual({ status: 'skipped', reason: 'scanning' });
    expect(apiMock.scan).toHaveBeenCalledTimes(1);
    releaseActivation({ settings: { source_root: '/new-root', effective_root: '/new-root', scan_interval_seconds: 0, enabled_provider_ids: ['claude'], provider_lookback_days: {} }, scan: makeReport({ root: '/new-root' }) });
    await activation;
    expect(browser.scanning.value).toBe(false);
  });

  it('reports busy and failed mutations without hiding the error', async () => {
    let release!: (value: SessionSummary) => void;
    apiMock.renameSession.mockReturnValueOnce(new Promise<SessionSummary>((resolve) => { release = resolve; }));
    const pending = browser.rename('session-1', 'Waiting');
    const busy = await browser.pin('session-1', true);
    expect(busy.status).toBe('busy');
    release({ ...makeSession(), title: 'Waiting' });
    await pending;

    apiMock.renameSession.mockRejectedValueOnce(new Error('rename failed'));
    const failed = await browser.rename('session-1', 'Broken');
    expect(failed.status).toBe('error');
    expect(browser.mutationError.value).toBe('rename failed');
  });

  it('keeps a successful mutation successful when navigation hydration fails', async () => {
    apiMock.projects.mockRejectedValueOnce(new Error('navigation refresh failed'));

    const result = await browser.rename('session-1', 'Renamed');

    expect(result.status).toBe('success');
    expect(browser.mutationError.value).toBeNull();
    expect(browser.navigationError.value).toBe('navigation refresh failed');
  });

  it('keeps a successful mutation successful when search hydration fails', async () => {
    browser.search('needle');
    await vi.advanceTimersByTimeAsync(180);
    await settle();
    apiMock.search.mockRejectedValueOnce(new Error('search refresh failed'));

    const result = await browser.rename('session-1', 'Renamed');

    expect(result.status).toBe('success');
    expect(browser.mutationError.value).toBeNull();
    expect(browser.navigationError.value).toBe('search refresh failed');
  });

  it('shows a content-free notice when source sessions are removed', async () => {
    apiMock.scan.mockResolvedValue(makeReport({ removed_sessions: 3 }));
    await browser.refresh();

    expect(browser.sourceRemovalNotice.value).toContain('3 sessions were removed');
    expect(browser.sourceRemovalNotice.value).toContain('different from hiding');
    expect(browser.sourceRemovalNotice.value).not.toContain('/tmp');
  });

  it('keeps navigation and current detail when a partial scan is not committed', async () => {
    const session = makeSession();
    await browser.refresh();
    await browser.select(session.id);
    apiMock.scan.mockResolvedValueOnce(makeReport({ partial: true, committed: false, outcome: 'partial', removed_sessions: 4 }));
    const result = await browser.refresh('scheduled');
    expect(result.status).toBe('success');
    expect(browser.projects.value[0]?.sessions[0]?.id).toBe(session.id);
    expect(browser.selected.value?.summary.id).toBe(session.id);
    expect(browser.sourceRemovalNotice.value).toBeNull();
  });

  it('hydrates the existing index when the startup scan is not committed', async () => {
    const descriptors = [{
      provider_id: 'claude',
      name: 'Claude',
      alias: 'Claude',
      capabilities: { supports_resume: true, supports_fork: true },
    }];
    apiMock.listProviderDescriptors.mockResolvedValueOnce(descriptors);
    apiMock.scan.mockResolvedValueOnce(makeReport({ partial: true, committed: false, outcome: 'partial' }));

    await browser.refresh();

    expect(browser.projects.value).toHaveLength(1);
    expect(browser.providerDescriptors.value).toEqual(descriptors);
  });

  it('returns a visible skipped result without hydrating an index', async () => {
    apiMock.scan.mockResolvedValueOnce(makeReport({ outcome: 'skipped_lifecycle', committed: false }));
    const result = await browser.refresh('scheduled');
    expect(result).toEqual({ status: 'skipped', reason: 'skipped_lifecycle' });
    expect(browser.scanNotice.value).toContain('Automatic scan skipped');
    expect(apiMock.projects).not.toHaveBeenCalled();
  });

  it('hydrates after root activation without starting a second scan', async () => {
    await browser.refresh();
    const scans = apiMock.scan.mock.calls.length;
    await browser.hydrateNavigation();
    expect(apiMock.scan).toHaveBeenCalledTimes(scans);
    expect(apiMock.projects).toHaveBeenCalledTimes(2);
  });

  it('loads an alternate branch and ignores a stale branch response after switching sessions', async () => {
    const session = makeSession();
    const branches = [makeBranch('main'), makeBranch('alt')];
    apiMock.session.mockResolvedValue({ ...makeDetail(session), branches, active_branch_id: 'main', selected_branch_id: 'main' });
    await browser.select('session-1');
    let release!: (detail: SessionDetail) => void;
    apiMock.getSessionBranch.mockReturnValueOnce(new Promise<SessionDetail>((resolve) => { release = resolve; }));
    const pending = browser.selectBranch('alt');
    expect(browser.branchLoading.value).toBe(true);
    await browser.select('session-2');
    release({ ...makeDetail(session), branches, active_branch_id: 'main', selected_branch_id: 'alt' });
    await pending;
    expect(browser.selectedId.value).toBe('session-2');
    expect(browser.selected.value?.selected_branch_id).not.toBe('alt');
    expect(browser.branchLoading.value).toBe(false);
  });

  it('exposes branch errors without replacing the current branch', async () => {
    const session = makeSession();
    apiMock.session.mockResolvedValue({ ...makeDetail(session), branches: [makeBranch('main'), makeBranch('alt')] , active_branch_id: 'main', selected_branch_id: 'main' });
    await browser.select('session-1');
    apiMock.getSessionBranch.mockRejectedValueOnce(new Error('branch unavailable'));
    await browser.selectBranch('alt');
    expect(browser.branchError.value).toBe('branch unavailable');
    expect(browser.selected.value?.selected_branch_id).toBe('main');
  });

  it('re-reads the selected session and branch after a successful scan', async () => {
    const session = makeSession();
    const branches = [makeBranch('main'), makeBranch('alt')];
    const initial = { ...makeDetail(session), branches, active_branch_id: 'main', selected_branch_id: 'main' };
    const refreshed = { ...makeDetail(session), branches, active_branch_id: 'main', selected_branch_id: 'main' };
    const refreshedBranch = { ...makeDetail(session), branches, active_branch_id: 'main', selected_branch_id: 'alt' };
    apiMock.session.mockResolvedValueOnce(initial).mockResolvedValueOnce(refreshed);
    apiMock.getSessionBranch.mockResolvedValueOnce({ ...initial, selected_branch_id: 'alt' }).mockResolvedValueOnce(refreshedBranch);

    await browser.select('session-1');
    await browser.selectBranch('alt');
    await browser.refresh();

    expect(apiMock.session).toHaveBeenCalledTimes(2);
    expect(apiMock.getSessionBranch).toHaveBeenNthCalledWith(2, 'session-1', 'alt');
    expect(browser.selected.value?.selected_branch_id).toBe('alt');
  });

  it('invalidates a pending branch response when a scan fails', async () => {
    const session = makeSession();
    const branches = [makeBranch('main'), makeBranch('alt')];
    apiMock.session.mockResolvedValue({ ...makeDetail(session), branches, active_branch_id: 'main', selected_branch_id: 'main' });
    await browser.select('session-1');

    let release!: (detail: SessionDetail) => void;
    apiMock.getSessionBranch.mockReturnValueOnce(new Promise<SessionDetail>((resolve) => { release = resolve; }));
    const pending = browser.selectBranch('alt');
    apiMock.scan.mockRejectedValueOnce(new Error('scan failed'));
    await browser.refresh();
    release({ ...makeDetail(session), branches, active_branch_id: 'main', selected_branch_id: 'alt' });
    await pending;

    expect(browser.selected.value?.selected_branch_id).toBe('main');
    expect(browser.branchError.value).toBeNull();
    expect(browser.branchLoading.value).toBe(false);
  });
});
