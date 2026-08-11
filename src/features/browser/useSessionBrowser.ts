import { computed, onScopeDispose, shallowReadonly, shallowRef } from 'vue';
import { api } from '../../api';
import type { Project, ProviderDescriptor, ProviderId, ScanReport, ScanTrigger, SearchHit, SessionDetail, SessionSummary, SourceRootActivationReport } from '../../types';
import { sortSessions } from './navigation';
import { scrollToReaderEvent } from '../reader/readerSearch';
import { useI18n, type MessageParams, type TranslationKey } from '../../i18n';

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'An unexpected local error occurred.';
}

function replaceSummary(projects: Project[], summary: SessionSummary): Project[] {
  const next = projects.map((project) => {
    const sessions = project.sessions.filter((session) => session.id !== summary.id);
    const agents = project.agents?.map((agent) => ({
      ...agent,
      sessions: agent.sessions.filter((session) => session.id !== summary.id),
    }));
    const projectWorkspace = project.workspace_id || project.id;
    const summaryWorkspace = summary.workspace_id || summary.project_id;
    const sameProject = summary.project_path
      ? summary.project_path === project.path
      : projectWorkspace === summaryWorkspace;
    if (!sameProject || summary.hidden) return { ...project, sessions, ...(agents ? { agents } : {}) };
    const nextSessions = sortSessions([...sessions, summary]);
    const nextAgents = agents?.map((agent) => agent.provider_id === summary.provider_id
      ? { ...agent, sessions: sortSessions([...agent.sessions, summary]) }
      : agent);
    return { ...project, sessions: nextSessions, ...(nextAgents ? { agents: nextAgents } : {}) };
  });
  return next.filter((project) => project.sessions.length > 0);
}

const mutationBusyError = 'Another session update is already in progress.';
type ScanOwnerKind = 'scan' | 'activation';
type Notice = {
  key: TranslationKey;
  params?: MessageParams;
  counts?: readonly [TranslationKey, number][];
};

export type SessionMutationResult =
  | { status: 'success'; summary: SessionSummary }
  | { status: 'busy'; error: string }
  | { status: 'error'; error: string };

export type SessionBrowserRefreshResult =
  | { status: 'success'; report: ScanReport }
  | { status: 'skipped'; reason: string }
  | { status: 'error'; error: string };

export function useSessionBrowser() {
  const { t } = useI18n();
  const projects = shallowRef<Project[]>([]);
  const hiddenSessions = shallowRef<SessionSummary[]>([]);
  const selectedId = shallowRef<string | null>(null);
  const detail = shallowRef<SessionDetail | null>(null);
  const query = shallowRef('');
  const hits = shallowRef<SearchHit[]>([]);
  const scanning = shallowRef(false);
  const hiddenLoading = shallowRef(false);
  const detailLoading = shallowRef(false);
  const branchLoading = shallowRef(false);
  const searchLoading = shallowRef(false);
  const mutationLoading = shallowRef<string | null>(null);
  const scanError = shallowRef<string | null>(null);
  const hiddenError = shallowRef<string | null>(null);
  const searchError = shallowRef<string | null>(null);
  const detailError = shallowRef<string | null>(null);
  const branchError = shallowRef<string | null>(null);
  const mutationError = shallowRef<string | null>(null);
  const aliasLoading = shallowRef<string | null>(null);
  const aliasError = shallowRef<string | null>(null);
  const partial = shallowRef(false);
  const sourceRemovalMessage = shallowRef<Notice | null>(null);
  const scanNoticeMessage = shallowRef<Notice | null>(null);
  const lastScanReport = shallowRef<ScanReport | null>(null);
  const providerDescriptors = shallowRef<ProviderDescriptor[]>([]);
  const providerId = shallowRef<ProviderId | null>(null);

  function renderNotice(notice: Notice | null): string | null {
    if (!notice) return null;
    const params = notice.counts
      ? { ...notice.params, counts: notice.counts.map(([key, value]) => `${t(key)} ${value}`).join(' · ') }
      : notice.params;
    return t(notice.key, params);
  }

  const sourceRemovalNotice = computed(() => renderNotice(sourceRemovalMessage.value));
  const scanNotice = computed(() => renderNotice(scanNoticeMessage.value));
  function setScanNotice(key: TranslationKey, params?: MessageParams, counts?: readonly [TranslationKey, number][]) {
    scanNoticeMessage.value = { key, params, counts };
  }

  let detailRequest = 0;
  let branchRequest = 0;
  let navigationRequest = 0;
  let hiddenRequest = 0;
  let searchRequest = 0;
  let refreshGeneration = 0;
  let noticeGeneration = 0;
  let scanOwner = false;
  let scanOwnerKind: ScanOwnerKind | null = null;
  let scanLeaseReserved = false;
  let scanIdleWaiters: Array<{ kind: ScanOwnerKind; resolve: () => void }> = [];
  let searchTimer: number | undefined;

  const navigationError = computed(() => scanError.value ?? searchError.value);
  const sortedHiddenSessions = computed(() => sortSessions(hiddenSessions.value));
  const effectiveHiddenLoading = computed(() => scanning.value || hiddenLoading.value);
  function acquireScan(kind: ScanOwnerKind = 'scan'): void {
    scanOwner = true;
    scanOwnerKind = kind;
    scanning.value = true;
  }
  function handoffScanLease(): void {
    const next = scanIdleWaiters.shift();
    if (next) {
      // Keep the busy gate asserted while ownership moves to the next
      // waiter. This avoids a false->true window between serialized scans.
      scanLeaseReserved = true;
      scanOwnerKind = next.kind;
      next.resolve();
      return;
    }
    scanOwnerKind = null;
    scanning.value = false;
  }
  function takeScanLease(kind: ScanOwnerKind = 'scan'): void {
    scanLeaseReserved = false;
    acquireScan(kind);
  }
  function cancelScanLease(): void {
    if (!scanLeaseReserved) return;
    scanLeaseReserved = false;
    handoffScanLease();
  }
  function releaseScan(generation: number): void {
    if (!scanOwner || generation !== refreshGeneration) return;
    scanOwner = false;
    handoffScanLease();
  }
  function waitForScanIdle(kind: ScanOwnerKind = 'scan'): Promise<boolean> {
    if (!scanOwner && !scanLeaseReserved && !scanning.value) return Promise.resolve(false);
    return new Promise((resolve) => scanIdleWaiters.push({ kind, resolve: () => resolve(true) }));
  }

  async function loadHiddenSessions() {
    const request = ++hiddenRequest;
    hiddenLoading.value = true;
    hiddenError.value = null;
    try {
      const nextSessions = await api.listHiddenSessions();
      if (request === hiddenRequest) hiddenSessions.value = sortSessions(nextSessions);
    } catch (error) {
      if (request === hiddenRequest) hiddenError.value = errorMessage(error);
    } finally {
      if (request === hiddenRequest) hiddenLoading.value = false;
    }
  }

  async function refreshNavigation() {
    const request = ++navigationRequest;
    scanError.value = null;
    try {
      const descriptors = typeof api.listProviderDescriptors === 'function'
        ? api.listProviderDescriptors()
        : Promise.resolve([] as ProviderDescriptor[]);
      const [nextProjects, _hidden, nextDescriptors] = await Promise.all([
        api.projects(),
        loadHiddenSessions(),
        descriptors,
      ]);
      if (request !== navigationRequest) return;
      providerDescriptors.value = nextDescriptors;
      projects.value = nextProjects.map((project) => ({
        ...project,
        sessions: sortSessions(project.sessions.filter((session) => !session.hidden)),
        agents: project.agents?.map((agent) => ({ ...agent, sessions: sortSessions(agent.sessions.filter((session) => !session.hidden)) })),
      }));
    } catch (error) {
      if (request === navigationRequest) scanError.value = errorMessage(error);
      throw error;
    }
  }

  async function refresh(trigger: ScanTrigger = 'manual'): Promise<SessionBrowserRefreshResult> {
    let generation = refreshGeneration;
    const current = () => generation === refreshGeneration;
    let noticeToken = noticeGeneration;
    let ownsScanning = false;
    let preloadedReport: ScanReport | null = null;
    if (scanning.value) {
      if (trigger !== 'scheduled') return { status: 'skipped', reason: 'scanning' };
      if (scanOwnerKind === 'activation' || scanLeaseReserved) {
        // Activation owns the backend transaction and its follow-up hydrate.
        // Delay the scheduled API call until activation releases the lease so
        // the scheduled report is unambiguously newer when it really starts.
        const leaseReserved = await waitForScanIdle('scan');
        if (!current()) {
          if (leaseReserved) cancelScanLease();
          return { status: 'skipped', reason: 'stale' };
        }
        generation = ++refreshGeneration;
        noticeToken = ++noticeGeneration;
        ownsScanning = true;
        takeScanLease('scan');
      } else {
        noticeToken = ++noticeGeneration;
        try {
          preloadedReport = await api.scan(trigger);
          if (!current()) return { status: 'skipped', reason: 'stale' };
          if (preloadedReport.outcome === 'skipped_lifecycle') {
            if (noticeToken === noticeGeneration) {
              lastScanReport.value = preloadedReport;
              setScanNotice('scheduledScanSkippedNoTime');
            }
            return { status: 'skipped', reason: 'skipped_lifecycle' };
          }
          // The scheduled scan completed a real commit while another UI
          // refresh owned the gate. Take ownership only now, after the backend
          // has proved this refresh must hydrate the committed index.
          if (preloadedReport.committed === true) {
            const leaseReserved = await waitForScanIdle('scan');
            if (!current()) {
              if (leaseReserved) cancelScanLease();
              return { status: 'skipped', reason: 'stale' };
            }
            // A newer activation report may have committed while this
            // scheduled report waited for the lease. Do not hydrate an older
            // index snapshot after that newer report has already won.
            if (noticeToken !== noticeGeneration) {
              if (leaseReserved) cancelScanLease();
              return { status: 'skipped', reason: 'stale' };
            }
            generation = ++refreshGeneration;
            ownsScanning = true;
            takeScanLease('scan');
          } else {
            // A partial/failed scheduled attempt is an observation, not a new
            // index generation. Do not invalidate A's detail/navigation work.
            if (noticeToken === noticeGeneration) {
              lastScanReport.value = preloadedReport;
              partial.value = preloadedReport.partial || preloadedReport.outcome === 'partial';
              setScanNotice(
                preloadedReport.partial ? 'scanIncomplete' : 'scanNotCommitted',
                preloadedReport.partial ? undefined : { outcome: preloadedReport.outcome },
              );
            }
            return { status: 'success', report: preloadedReport };
          }
        } catch (error) {
          return { status: 'error', error: errorMessage(error) };
        }
      }
    } else {
      generation = ++refreshGeneration;
      noticeToken = ++noticeGeneration;
      ownsScanning = true;
      acquireScan('scan');
    }

    const activeId = selectedId.value;
    const activeBranchId = detail.value?.selected_branch_id ?? null;
    // A scan can overlap an in-flight detail/branch request. Invalidate both
    // generations before touching the index so late responses cannot restore
    // content from the pre-scan snapshot.
    detailRequest += 1;
    branchRequest += 1;
    detailLoading.value = false;
    branchLoading.value = false;
    if (!scanOwner) acquireScan();
    scanError.value = null;
    if (noticeToken === noticeGeneration) {
      sourceRemovalMessage.value = null;
      scanNoticeMessage.value = null;
    }
    try {
      const report = preloadedReport ?? await api.scan(trigger);
      if (!current()) return { status: 'skipped', reason: 'stale' };
      if (noticeToken === noticeGeneration) lastScanReport.value = report;
      const committed = report.committed;
      if (noticeToken === noticeGeneration) partial.value = report.partial || report.outcome === 'partial';
      const outcome = report.outcome;
      if (outcome === 'skipped_lifecycle') {
        if (noticeToken === noticeGeneration) {
          setScanNotice('scheduledScanSkippedNoTime');
        }
        return { status: 'skipped', reason: 'skipped_lifecycle' };
      }
      if (!committed) {
        if (!current()) return { status: 'skipped', reason: 'stale' };
        await refreshNavigation();
        if (!current()) return { status: 'skipped', reason: 'stale' };
        if (noticeToken === noticeGeneration) {
          setScanNotice(
            report.partial ? 'scanIncomplete' : 'scanNotCommitted',
            report.partial ? undefined : { outcome },
          );
        }
        return { status: 'success', report };
      }
      await refreshNavigation();
      if (!current()) return { status: 'skipped', reason: 'stale' };
      if (noticeToken === noticeGeneration && report.removed_sessions && report.removed_sessions > 0) {
        sourceRemovalMessage.value = { key: 'sourceRemoved', params: { count: report.removed_sessions } };
      }
      const counts = [
        ['newCount', report.new_files],
        ['updatedCount', report.changed_files],
        ['unchangedCount', report.unchanged_files],
        ['removedCount', report.removed_files],
        ['partialCount', report.partial_sessions],
      ].filter(([, value]) => typeof value === 'number' && value > 0) as [TranslationKey, number][];
      if (noticeToken === noticeGeneration && counts.length) setScanNotice('scanCompleted', undefined, counts);
      await refreshCurrentSearch(generation);
      if (!current()) return { status: 'skipped', reason: 'stale' };

      if (
        activeId &&
        !projects.value.some((project) => project.sessions.some((session) => session.id === activeId))
      ) {
        detailRequest += 1;
        selectedId.value = null;
        detail.value = null;
        detailError.value = null;
      } else if (activeId && selectedId.value === activeId) {
        // Re-read the current session after a successful scan. If the user was
        // viewing an alternate branch, restore it only when the scan still
        // reports that branch; otherwise retain the canonical active detail.
        await select(activeId);
        if (
          activeBranchId &&
          selectedId.value === activeId &&
          detail.value?.branches?.some((branch) => branch.id === activeBranchId) &&
          detail.value?.selected_branch_id !== activeBranchId
        ) {
          await selectBranch(activeBranchId);
        }
      }
      if (!current()) return { status: 'skipped', reason: 'stale' };
      return { status: 'success', report };
    } catch (error) {
      const message = errorMessage(error);
      if (!current()) return { status: 'skipped', reason: 'stale' };
      if (current()) scanError.value = message;
      return { status: 'error', error: message };
    } finally {
      if (ownsScanning && current()) releaseScan(generation);
    }
  }

  /** Rehydrate navigation/search after an already-committed root activation.
   * This deliberately does not invoke scan a second time. */
  async function hydrateNavigation(): Promise<void> {
    const activeId = selectedId.value;
    const activeBranchId = detail.value?.selected_branch_id ?? null;
    detailRequest += 1;
    branchRequest += 1;
    try {
      await refreshNavigation();
      await refreshCurrentSearch();
      if (!activeId) return;
      if (!projects.value.some((project) => project.sessions.some((session) => session.id === activeId))) {
        selectedId.value = null;
        detail.value = null;
        detailError.value = null;
        return;
      }
      await select(activeId);
      if (activeBranchId && detail.value?.branches?.some((branch) => branch.id === activeBranchId) && detail.value.selected_branch_id !== activeBranchId) {
        await selectBranch(activeBranchId);
      }
    } catch (error) {
      scanError.value = errorMessage(error);
    }
  }

  function setScanReportNotice(report: ScanReport, token: number): void {
    if (token !== noticeGeneration) return;
    lastScanReport.value = report;
    partial.value = report.partial || report.outcome === 'partial';
    if (report.committed && report.removed_sessions > 0) {
      sourceRemovalMessage.value = { key: 'sourceRemoved', params: { count: report.removed_sessions } };
    } else if (report.committed) {
      sourceRemovalMessage.value = null;
    }
    if (!report.committed) {
      setScanNotice(
        report.partial ? 'scanIncomplete' : 'scanNotCommitted',
        report.partial ? undefined : { outcome: report.outcome },
      );
      return;
    }
    if (report.partial) {
      setScanNotice('scanCommittedPartial');
      return;
    }
    const counts = [
      ['newCount', report.new_files], ['updatedCount', report.changed_files], ['unchangedCount', report.unchanged_files],
      ['removedCount', report.removed_files], ['partialCount', report.partial_sessions],
    ].filter(([, value]) => Number(value) > 0) as [TranslationKey, number][];
    if (counts.length) setScanNotice('scanCompleted', undefined, counts);
    else scanNoticeMessage.value = null;
  }

  async function hydrateCommittedReport(report: ScanReport, generation: number, token: number): Promise<void> {
    setScanReportNotice(report, token);
    await hydrateNavigation();
    if (generation !== refreshGeneration) return;
  }

  async function runSourceRootActivation(
    sourceRoot: string | null,
    replaceActiveIndexAcknowledged: boolean,
    generation: number,
  ): Promise<SourceRootActivationReport> {
    try {
      const activation = await api.activateClaudeSourceRoot(sourceRoot, replaceActiveIndexAcknowledged);
      // The report is the newest committed observation only when the backend
      // returns it; scheduled skip observations during the activation must not
      // hide the activation result.
      const token = ++noticeGeneration;
      if (activation.scan.committed === true) {
        await hydrateCommittedReport(activation.scan, generation, token);
      } else {
        setScanReportNotice(activation.scan, token);
      }
      return activation;
    } finally {
      releaseScan(generation);
    }
  }

  async function activateSourceRoot(sourceRoot: string | null, replaceActiveIndexAcknowledged: boolean): Promise<SourceRootActivationReport> {
    // Reserve an idle lease synchronously so a refresh cannot slip in during
    // the microtask before an activation's async body starts.
    if (!scanOwner && !scanLeaseReserved && !scanning.value) {
      const generation = ++refreshGeneration;
      acquireScan('activation');
      return runSourceRootActivation(sourceRoot, replaceActiveIndexAcknowledged, generation);
    }
    await waitForScanIdle('activation');
    const generation = ++refreshGeneration;
    takeScanLease('activation');
    return runSourceRootActivation(sourceRoot, replaceActiveIndexAcknowledged, generation);
  }

  function updateDetailSummary(summary: SessionSummary) {
    if (detail.value?.summary.id === summary.id) {
      detail.value = { ...detail.value, summary };
    }
  }

  async function refreshCurrentSearch(generation?: number): Promise<void> {
    const trimmed = query.value.trim();
    window.clearTimeout(searchTimer);
    const request = ++searchRequest;
    if (!trimmed) {
      hits.value = [];
      searchError.value = null;
      searchLoading.value = false;
      return;
    }

    searchLoading.value = true;
    searchError.value = null;
    try {
      const nextHits = await api.search(trimmed);
      if (request === searchRequest && (generation === undefined || generation === refreshGeneration)) hits.value = nextHits;
    } catch (error) {
      if (request === searchRequest) searchError.value = errorMessage(error);
    } finally {
      if (request === searchRequest) searchLoading.value = false;
    }
  }

  async function select(id: string, eventId?: number) {
    const request = ++detailRequest;
    branchRequest += 1;
    selectedId.value = id;
    detailLoading.value = true;
    branchLoading.value = false;
    detailError.value = null;
    branchError.value = null;
    mutationError.value = null;

    try {
      const nextDetail = await api.session(id);
      if (request !== detailRequest) return;

      detail.value = nextDetail;
      if (eventId && eventId > 0) {
        window.requestAnimationFrame(() => {
          scrollToReaderEvent(eventId);
        });
      }
    } catch (error) {
      if (request === detailRequest) detailError.value = errorMessage(error);
    } finally {
      if (request === detailRequest) detailLoading.value = false;
    }
  }

  /** Load a read-only alternate branch while preventing stale responses from
   * replacing a newer session or branch selection. */
  async function selectBranch(branchId: string) {
    const sessionId = selectedId.value;
    const current = detail.value;
    if (!sessionId || !current || !branchId || branchId === current.selected_branch_id) return;

    // The initial session payload is the canonical active branch. Reusing it
    // avoids a redundant request while still marking the branch explicitly.
    if (branchId === current.active_branch_id && !current.selected_branch_id) {
      detail.value = { ...current, selected_branch_id: branchId };
      return;
    }

    const request = ++branchRequest;
    branchLoading.value = true;
    branchError.value = null;
    try {
      const nextDetail = await api.getSessionBranch(sessionId, branchId);
      if (request !== branchRequest || selectedId.value !== sessionId) return;
      detail.value = {
        ...nextDetail,
        branches: nextDetail.branches ?? current.branches,
        active_branch_id: nextDetail.active_branch_id ?? current.active_branch_id,
        selected_branch_id: nextDetail.selected_branch_id ?? branchId,
      };
    } catch (error) {
      if (request === branchRequest && selectedId.value === sessionId) branchError.value = errorMessage(error);
    } finally {
      if (request === branchRequest) branchLoading.value = false;
    }
  }

  async function mutate(
    sessionId: string,
    operation: () => Promise<SessionSummary>,
  ): Promise<SessionMutationResult> {
    if (mutationLoading.value) {
      mutationError.value = mutationBusyError;
      return { status: 'busy', error: mutationBusyError };
    }
    mutationLoading.value = sessionId;
    mutationError.value = null;
    try {
      const summary = await operation();
      updateDetailSummary(summary);
      projects.value = replaceSummary(projects.value, summary);
      hiddenSessions.value = summary.hidden
        ? sortSessions([...hiddenSessions.value.filter((session) => session.id !== summary.id), summary])
        : hiddenSessions.value.filter((session) => session.id !== summary.id);
      if (summary.hidden && selectedId.value === summary.id) {
        detailRequest += 1;
        selectedId.value = null;
        detail.value = null;
        detailError.value = null;
      }
      try {
        await refreshNavigation();
      } catch {
        // refreshNavigation records hydration errors on the navigation surface.
      }
      await refreshCurrentSearch();
      return { status: 'success', summary };
    } catch (error) {
      const message = errorMessage(error);
      mutationError.value = message;
      return { status: 'error', error: message };
    } finally {
      if (mutationLoading.value === sessionId) mutationLoading.value = null;
    }
  }

  const hide = (sessionId: string) => mutate(sessionId, () => api.setSessionHidden(sessionId, true));
  const restore = (sessionId: string) => mutate(sessionId, () => api.setSessionHidden(sessionId, false));
  const rename = (sessionId: string, title: string | null) => mutate(sessionId, () => api.renameSession(sessionId, title));
  const pin = (sessionId: string, pinned: boolean) => mutate(sessionId, () => api.setSessionPinned(sessionId, pinned));

  async function setProjectAlias(providerIdOrWorkspaceId: ProviderId | string, workspaceIdOrAlias: string | null, nextAlias?: string | null): Promise<boolean> {
    const workspaceId = nextAlias === undefined ? providerIdOrWorkspaceId : workspaceIdOrAlias as string;
    const alias = nextAlias === undefined ? workspaceIdOrAlias : nextAlias;
    const providerId = nextAlias === undefined
      ? projects.value.find((project) => (project.workspace_id || project.id) === workspaceId)?.agents?.[0]?.provider_id
        ?? projects.value.find((project) => (project.workspace_id || project.id) === workspaceId)?.provider_ids?.[0]
        ?? projects.value.find((project) => (project.workspace_id || project.id) === workspaceId)?.sessions[0]?.provider_id
      : providerIdOrWorkspaceId;
    if (!providerId) return false;
    if (aliasLoading.value) return false;
    aliasLoading.value = workspaceId;
    aliasError.value = null;
    const navigationToken = ++navigationRequest;
    try {
      const nextProjects = await api.setProjectAlias(providerId, workspaceId, alias);
      if (navigationToken !== navigationRequest) return false;
      projects.value = nextProjects.map((project) => ({
        ...project,
        sessions: sortSessions(project.sessions.filter((session) => !session.hidden)),
        agents: project.agents?.map((agent) => ({ ...agent, sessions: sortSessions(agent.sessions.filter((session) => !session.hidden)) })),
      })).filter((project) => project.sessions.length > 0);
      return true;
    } catch (error) {
      aliasError.value = errorMessage(error);
      return false;
    } finally {
      if (aliasLoading.value === workspaceId) aliasLoading.value = null;
    }
  }

  function search(value: string) {
    query.value = value;
    searchError.value = null;
    hits.value = [];
    window.clearTimeout(searchTimer);

    const trimmed = value.trim();
    const request = ++searchRequest;
    if (!trimmed) {
      searchLoading.value = false;
      return;
    }

    searchLoading.value = true;
    searchTimer = window.setTimeout(async () => {
      try {
        const nextHits = providerId.value === null
          ? await api.search(trimmed)
          : await api.search(trimmed, providerId.value);
        if (request === searchRequest) hits.value = nextHits;
      } catch (error) {
        if (request === searchRequest) searchError.value = errorMessage(error);
      } finally {
        if (request === searchRequest) searchLoading.value = false;
      }
    }, 180);
  }

  function setProviderFilter(next: ProviderId | null): void {
    if (providerId.value === next) return;
    providerId.value = next;
    search(query.value);
  }

  onScopeDispose(() => {
    detailRequest += 1;
    branchRequest += 1;
    navigationRequest += 1;
    hiddenRequest += 1;
    searchRequest += 1;
    window.clearTimeout(searchTimer);
  });

  return {
    projects: shallowReadonly(projects),
    hiddenSessions: shallowReadonly(sortedHiddenSessions),
    selectedId: shallowReadonly(selectedId),
    selected: shallowReadonly(detail),
    query: shallowReadonly(query),
    hits: shallowReadonly(hits),
    providerDescriptors: shallowReadonly(providerDescriptors),
    providerId: shallowReadonly(providerId),
    scanning: shallowReadonly(scanning),
    hiddenLoading: effectiveHiddenLoading,
    detailLoading: shallowReadonly(detailLoading),
    branchLoading: shallowReadonly(branchLoading),
    searchLoading: shallowReadonly(searchLoading),
    mutationLoading: shallowReadonly(mutationLoading),
    navigationError,
    hiddenError: shallowReadonly(hiddenError),
    detailError: shallowReadonly(detailError),
    branchError: shallowReadonly(branchError),
    mutationError: shallowReadonly(mutationError),
    aliasLoading: shallowReadonly(aliasLoading),
    aliasError: shallowReadonly(aliasError),
    partial: shallowReadonly(partial),
    sourceRemovalNotice: shallowReadonly(sourceRemovalNotice),
    scanNotice: shallowReadonly(scanNotice),
    lastScanReport: shallowReadonly(lastScanReport),
    refresh,
    hydrateNavigation,
    activateSourceRoot,
    select,
    selectBranch,
    search,
    setProviderFilter,
    hide,
    restore,
    rename,
    pin,
    setProjectAlias,
  };
}
