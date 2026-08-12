<script setup lang="ts">
import { computed, onMounted, onUnmounted, shallowRef, watch } from 'vue';
import { api } from './api';
import NavigationPane from './features/browser/NavigationPane.vue';
import { useSessionBrowser } from './features/browser/useSessionBrowser';
import ContextReader from './features/reader/ContextReader.vue';
import { isEditableTarget } from './features/reader/useReaderKeyboard';
import ContinuationTerminal from './features/terminal/ContinuationTerminal.vue';
import { bindForkChild } from './features/terminal/continuationChildBinding';
import SettingsPanel from './features/settings/SettingsPanel.vue';
import { useScanSchedule } from './features/settings/useScanSchedule';
import { useKnowledge } from './features/knowledge/useKnowledge';
import { formatTime, useI18n } from './i18n';
import type { KnowledgeCardPatch } from './types';
import type { LiveTranscriptEvent, ProviderCapabilities, ScanReport, ScanSettings } from './types';
import type { ContinuationViewState } from './features/terminal/continuationTypes';

const {
  projects,
  hiddenSessions,
  selectedId,
  selected,
  query,
  hits,
  providerDescriptors,
  providerId,
  scanning,
  detailLoading,
  branchLoading,
  searchLoading,
  hiddenLoading,
  hiddenError,
  mutationLoading,
  mutationError,
  navigationError,
  detailError,
  branchError,
  partial,
  sourceRemovalNotice,
  scanNotice,
  aliasLoading,
  aliasError,
  setProjectAlias,
  refresh,
  hydrateNavigation,
  activateSourceRoot,
  select,
  selectBranch,
  search,
  hide,
  restore,
  rename,
  pin,
  setProviderFilter,
} = useSessionBrowser();
const { locale, t } = useI18n();
const {
  card: knowledgeCard,
  related: relatedKnowledge,
  loading: knowledgeLoading,
  saving: knowledgeSaving,
  error: knowledgeError,
  load: loadKnowledge,
  save: saveKnowledgeCard,
  semanticQuery,
  semanticScope,
  semanticResults,
  semanticLoading,
  semanticError,
  setSemanticQuery,
  setSemanticScope,
  searchSemantic,
} = useKnowledge();
const settingsOpen = shallowRef(false);
// The source stays locked for the lifetime of a PTY. Forks may report a new
// target session; both parent and child remain guarded until exit sync ends.
const continuationSessionId = shallowRef<string | null>(null);
const continuationTargetSessionId = shallowRef<string | null>(null);
const continuationMode = shallowRef<'resume' | 'fork'>('resume');
const continuationTitle = shallowRef('');
const continuationPhase = shallowRef<ContinuationViewState['phase']>('idle');
const continuationLiveEvents = shallowRef<readonly LiveTranscriptEvent[]>([]);
const continuationNoNewEvents = shallowRef(false);
const continuationTailPartial = shallowRef(false);
const continuationTailDiagnostics = shallowRef(0);
const continuationTailError = shallowRef<string | null>(null);
const continuationError = shallowRef<string | null>(null);
const scanIntervalSeconds = shallowRef(0);
const scanSettingsError = shallowRef<string | null>(null);
const schedule = useScanSchedule({ intervalSeconds: scanIntervalSeconds, scanning, continuationActive: computed(() => Boolean(continuationSessionId.value)), refresh });
const selectedProviderCapabilities = computed<ProviderCapabilities | null>(() => {
  const current = selected.value;
  if (!current) return null;
  return projects.value
    .find((project) => project.sessions.some((session) => session.id === current.summary.id))
    ?.agents?.find((agent) => agent.provider_id === current.summary.provider_id)?.capabilities
    ?? providerDescriptors.value.find((provider) => provider.provider_id === current.summary.provider_id)?.capabilities
    ?? null;
});
watch(
  () => ({
    sessionId: selected.value?.summary.id ?? null,
    providerId: selected.value?.summary.provider_id,
  }),
  ({ sessionId, providerId }) => { void loadKnowledge(sessionId, providerId); },
  { immediate: true },
);
function saveKnowledge(patch: KnowledgeCardPatch) { void saveKnowledgeCard(patch); }
function openRelatedSession(sessionId: string) { void select(sessionId); }
function runSemanticSearch() { void searchSemantic(selected.value?.summary.provider_id); }
let finishingContinuation = false;

function openContinuation(mode: 'resume' | 'fork' = 'resume') {
  if (continuationSessionId.value || scanning.value) return;
  const detail = selected.value;
  if (!detail) return;
  const session = detail.summary;
  if (!session.native_session_id || !session.cwd || !selectedProviderCapabilities.value?.supports_resume) return;
  // Re-check at the command boundary: queued/programmatic emits can arrive
  // after the button was rendered but while branch data is changing.
  if (branchLoading.value || !isCurrentActiveHead(detail)) return;
  continuationSessionId.value = session.id;
  continuationTargetSessionId.value = session.id;
  continuationMode.value = mode;
  continuationError.value = null;
  continuationTitle.value = session.title;
}

function isCurrentActiveHead(detail: NonNullable<typeof selected.value>): boolean {
  if (
    detail.selected_branch_id
    && detail.active_branch_id
    && detail.selected_branch_id !== detail.active_branch_id
  ) return false;
  const branches = detail.branches ?? [];
  if (!branches.length) return true;
  const selectedBranch = branches.find((item) => item.id === detail.selected_branch_id);
  return Boolean(
    detail.selected_branch_id
      && detail.selected_branch_id === detail.active_branch_id
      && selectedBranch?.is_active,
  );
}

// Session actions are disabled in the row UI as well as here. The second
// check closes the race where a continuation starts after a row event has
// already been queued but before the mutation reaches the command boundary.
function continuationAllowsMutation(sessionId: string): boolean {
  return sessionId !== continuationSessionId.value && sessionId !== continuationTargetSessionId.value;
}
function guardedHide(sessionId: string) {
  if (continuationAllowsMutation(sessionId)) void hide(sessionId);
}
function guardedRename(sessionId: string, title: string | null) {
  if (continuationAllowsMutation(sessionId)) void rename(sessionId, title);
}
function guardedPin(sessionId: string, value: boolean) {
  if (continuationAllowsMutation(sessionId)) void pin(sessionId, value);
}
function guardedRestore(sessionId: string) {
  if (sessionId === continuationSessionId.value || sessionId === continuationTargetSessionId.value) return;
  void restore(sessionId);
}
function updateContinuationState(state: ContinuationViewState) {
  continuationPhase.value = state.phase;
  continuationLiveEvents.value = state.liveEvents;
  continuationNoNewEvents.value = state.noNewEvents ?? false;
  continuationTailPartial.value = state.tailPartial;
  continuationTailDiagnostics.value = state.tailDiagnostics;
  continuationTailError.value = state.tailError;
  continuationError.value = state.error;
  continuationTargetSessionId.value = bindForkChild(
    continuationMode.value,
    continuationSessionId.value,
    continuationTargetSessionId.value,
    state,
  );
}
async function finishContinuation() {
  if (finishingContinuation) return;
  finishingContinuation = true;
  const sourceSessionId = continuationSessionId.value;
  const targetSessionId = continuationTargetSessionId.value;
  const mode = continuationMode.value;
  if (!sourceSessionId) {
    finishingContinuation = false;
    return;
  }
  let missingFork = false;
  let finishError: string | null = null;
  try {
    const refreshResult = await refresh('post_continuation');
    if (refreshResult.status !== 'success' || refreshResult.report.committed !== true) {
      const reason = refreshResult.status === 'error'
        ? refreshResult.error
        : refreshResult.status === 'skipped'
          ? t('scanSkipped', { value: refreshResult.reason })
          : t('scanNotCommitted', { outcome: refreshResult.report.outcome });
      finishError = t('scanSyncIncomplete', { value: reason });
      return;
    }
    const targetExists = Boolean(targetSessionId && projects.value.some((project) => project.sessions.some((session) => session.id === targetSessionId)));
    if (mode === 'fork') {
      if (targetExists && targetSessionId) await select(targetSessionId);
      else {
        missingFork = true;
        continuationError.value = t('forkFinishedNoSource');
      }
    } else {
      await select(sourceSessionId);
    }
  } finally {
    continuationSessionId.value = null;
    continuationTargetSessionId.value = null;
    continuationMode.value = 'resume';
    continuationTitle.value = '';
    continuationPhase.value = 'idle';
    continuationLiveEvents.value = [];
    continuationNoNewEvents.value = false;
    continuationTailPartial.value = false;
    continuationTailDiagnostics.value = 0;
    continuationTailError.value = null;
    if (finishError) continuationError.value = finishError;
    else if (!missingFork) continuationError.value = null;
    finishingContinuation = false;
  }
}

function focusSearch(event: KeyboardEvent) {
  if (event.isComposing || isEditableTarget(event.target)) return;
  if (settingsOpen.value) return;
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
    event.preventDefault();
    document.querySelector<HTMLInputElement>('.search-box input')?.focus();
  }
}

async function handleRootActivated(report: ScanReport) {
  // The browser owns activation's scan lease and has already hydrated this
  // committed report before Settings emits the UI notification.
  void report;
}

function handleScanSettingsChanged(settings: ScanSettings, scanScopeChanged: boolean) {
  scanIntervalSeconds.value = settings.scan_interval_seconds;
  schedule.reschedule(settings.scan_interval_seconds);
  if (scanScopeChanged) void refresh('manual');
}

onMounted(() => {
  void hydrateNavigation().then(() => refresh('manual'));
  void api.getScanSettings().then((settings) => {
    scanSettingsError.value = null;
    scanIntervalSeconds.value = settings.scan_interval_seconds;
    schedule.reschedule(settings.scan_interval_seconds);
  }).catch((cause) => { scanSettingsError.value = cause instanceof Error ? cause.message : String(cause); });
  window.addEventListener('keydown', focusSearch);
});

onUnmounted(() => window.removeEventListener('keydown', focusSearch));
</script>

<template>
  <div class="app-shell" :lang="locale === 'zh' ? 'zh-CN' : 'en'">
    <NavigationPane
      :projects="projects"
      :selected-id="selectedId"
      :query="query"
      :hits="hits"
      :providers="providerDescriptors"
      :provider-id="providerId"
      :scanning="scanning"
      :searching="searchLoading"
      :error="navigationError"
      :hidden-sessions="hiddenSessions"
      :hidden-loading="hiddenLoading"
      :hidden-error="hiddenError"
      :mutation-loading="mutationLoading"
      :mutation-error="mutationError"
      :alias-busy="aliasLoading"
      :active-continuation-session-id="continuationSessionId"
      :active-continuation-target-session-id="continuationTargetSessionId"
      :continuation-active="Boolean(continuationSessionId)"
      @search="search"
      @select="select"
      @hide="guardedHide"
      @restore="guardedRestore"
      @rename="guardedRename"
      @alias="setProjectAlias"
      @provider="setProviderFilter"
      @pin="guardedPin"
      @refresh="refresh()"
      @settings="settingsOpen = true"
    />
    <section class="reader-column">
      <ContextReader
        :detail="selected"
        :provider-capabilities="selectedProviderCapabilities"
        :knowledge-card="knowledgeCard"
        :knowledge-loading="knowledgeLoading"
        :knowledge-saving="knowledgeSaving"
        :knowledge-error="knowledgeError"
        :related-sessions="relatedKnowledge"
        :related-loading="knowledgeLoading"
        :related-error="knowledgeError"
        :semantic-query="semanticQuery"
        :semantic-scope="semanticScope"
        :semantic-results="semanticResults"
        :semantic-loading="semanticLoading"
        :semantic-error="semanticError"
        :loading="detailLoading"
        :error="detailError"
        :branch-loading="branchLoading"
        :branch-error="branchError"
        :scanning="scanning"
        :active-continuation-session-id="continuationSessionId"
        :active-continuation-title="continuationTitle"
        :continuation-phase="continuationPhase"
        :continuation-error="continuationError"
        :continuation-no-new-events="continuationNoNewEvents"
        :live-events="continuationLiveEvents"
        :tail-partial="continuationTailPartial"
        :tail-diagnostics="continuationTailDiagnostics"
        :tail-error="continuationTailError"
        @select-branch="selectBranch"
        @continue="openContinuation('resume')"
        @fork="openContinuation('fork')"
        @save-knowledge="saveKnowledge"
        @open-related-session="openRelatedSession"
        @update-semantic-query="setSemanticQuery"
        @update-semantic-scope="setSemanticScope"
        @semantic-search="runSemanticSearch"
      />
      <ContinuationTerminal v-if="continuationSessionId" :session-id="continuationSessionId" :title="continuationTitle" :mode="continuationMode" @state-change="updateContinuationState" @closed="finishContinuation" />
    </section>
    <div class="scan-indicators">
      <div v-if="partial" class="scan-indicator" role="status">{{ t('partialScan') }}</div>
      <div v-if="scanNotice" class="scan-indicator" role="status">{{ scanNotice }}</div>
      <div v-if="schedule.skipReason.value" class="scan-indicator" role="status">
        {{ t('scheduledScanSkipped', { value: schedule.nextRunAt.value ? formatTime(schedule.nextRunAt.value) : t('later') }) }}
      </div>
      <div v-if="schedule.error.value" class="scan-indicator continuation-error" role="alert">{{ t('automaticScanFailed', { value: schedule.error.value }) }}</div>
      <div v-if="scanSettingsError" class="scan-indicator continuation-error" role="alert">{{ t('automaticSettingsUnavailable', { value: scanSettingsError }) }}</div>
      <div v-if="aliasError" class="scan-indicator continuation-error" role="alert">{{ t('projectAliasSaveFailed', { value: aliasError }) }}</div>
      <div v-if="sourceRemovalNotice" class="scan-indicator source-removal-notice" role="status">{{ sourceRemovalNotice }}</div>
      <div v-if="continuationError && !continuationSessionId" class="scan-indicator continuation-error" role="alert">{{ continuationError }}</div>
    </div>
    <SettingsPanel v-if="settingsOpen" :session="selected?.summary ?? null" :scanning="scanning" :continuation-active="Boolean(continuationSessionId)" :providers="providerDescriptors" :activate-source-root="activateSourceRoot" @close="settingsOpen = false" @root-activated="handleRootActivated" @scan-settings-changed="handleScanSettingsChanged" />
  </div>
</template>
