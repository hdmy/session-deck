<script setup lang="ts">
import { computed, onMounted, onUnmounted, shallowRef } from 'vue';
import { api } from './api';
import NavigationPane from './features/browser/NavigationPane.vue';
import { useSessionBrowser } from './features/browser/useSessionBrowser';
import ContextReader from './features/reader/ContextReader.vue';
import { isEditableTarget } from './features/reader/useReaderKeyboard';
import ContinuationTerminal from './features/terminal/ContinuationTerminal.vue';
import { bindForkChild } from './features/terminal/continuationChildBinding';
import SettingsPanel from './features/settings/SettingsPanel.vue';
import { useScanSchedule } from './features/settings/useScanSchedule';
import type { LiveTranscriptEvent, ScanReport } from './types';
import type { ContinuationViewState } from './features/terminal/continuationTypes';

const {
  projects,
  hiddenSessions,
  selectedId,
  selected,
  query,
  hits,
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
  activateSourceRoot,
  select,
  selectBranch,
  search,
  hide,
  restore,
  rename,
  pin,
} = useSessionBrowser();
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
let finishingContinuation = false;

function openContinuation(mode: 'resume' | 'fork' = 'resume') {
  if (continuationSessionId.value || scanning.value) return;
  const detail = selected.value;
  if (!detail) return;
  const session = detail.summary;
  if (!session.native_session_id || !session.cwd || session.provider_id !== 'claude') return;
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
          ? `扫描跳过（${refreshResult.reason}）`
          : `扫描未提交（${refreshResult.report.outcome}）`;
      finishError = `扫描/同步未完成：${reason}`;
      return;
    }
    const targetExists = Boolean(targetSessionId && projects.value.some((project) => project.sessions.some((session) => session.id === targetSessionId)));
    if (mode === 'fork') {
      if (targetExists && targetSessionId) await select(targetSessionId);
      else {
        missingFork = true;
        continuationError.value = '分叉已结束，但未发现新的 source；请重新扫描后检查 Claude 输出。';
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

onMounted(() => {
  void refresh('manual');
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
  <div class="app-shell">
    <NavigationPane
      :projects="projects"
      :selected-id="selectedId"
      :query="query"
      :hits="hits"
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
      @pin="guardedPin"
      @refresh="refresh()"
      @settings="settingsOpen = true"
    />
    <section class="reader-column">
      <ContextReader
        :detail="selected"
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
      />
      <ContinuationTerminal v-if="continuationSessionId" :session-id="continuationSessionId" :title="continuationTitle" :mode="continuationMode" @state-change="updateContinuationState" @closed="finishContinuation" />
    </section>
    <div v-if="partial" class="scan-indicator" role="status">Partial scan</div>
    <div v-if="scanNotice" class="scan-indicator" role="status">{{ scanNotice }}</div>
    <div v-if="schedule.skipReason.value" class="scan-indicator" role="status">自动扫描跳过：扫描或续聊进行中；下一次将在 {{ schedule.nextRunAt.value ? new Date(schedule.nextRunAt.value).toLocaleTimeString() : '稍后' }} 尝试。</div>
    <div v-if="schedule.error.value" class="scan-indicator continuation-error" role="alert">自动扫描失败：{{ schedule.error.value }}</div>
    <div v-if="scanSettingsError" class="scan-indicator continuation-error" role="alert">自动扫描设置不可用：{{ scanSettingsError }}</div>
    <div v-if="aliasError" class="scan-indicator continuation-error" role="alert">项目别名保存失败：{{ aliasError }}</div>
    <div v-if="sourceRemovalNotice" class="scan-indicator source-removal-notice" role="status">
      {{ sourceRemovalNotice }}
    </div>
    <div v-if="continuationError && !continuationSessionId" class="scan-indicator continuation-error" role="alert">
      {{ continuationError }}
    </div>
    <SettingsPanel v-if="settingsOpen" :session="selected?.summary ?? null" :scanning="scanning" :continuation-active="Boolean(continuationSessionId)" :activate-source-root="activateSourceRoot" @close="settingsOpen = false" @root-activated="handleRootActivated" @scan-settings-changed="(value) => { scanIntervalSeconds = value; schedule.reschedule(value); }" />
  </div>
</template>
