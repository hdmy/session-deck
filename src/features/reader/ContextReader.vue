<script setup lang="ts">
import { computed, onScopeDispose, shallowRef } from 'vue';
import type { LiveTranscriptEvent, SessionDetail, TurnInsight } from '../../types';
import TimelineEvent from './TimelineEvent.vue';
import LiveTranscriptPanel from './LiveTranscriptPanel.vue';
import { buildConversationTurns } from './conversationTurns';
import BranchSelector from './BranchSelector.vue';
import HistoryHint from './HistoryHint.vue';
import SessionSearchControls from './SessionSearchControls.vue';
import { useReaderKeyboard } from './useReaderKeyboard';
import { useReaderSearch } from './useReaderSearch';
import TurnChangeSummary from './TurnChangeSummary.vue';
import ToolSummary from './ToolSummary.vue';
import { copyText } from './copyText';

const props = defineProps<{
  detail: SessionDetail | null;
  loading: boolean;
  error: string | null;
  activeContinuationSessionId?: string | null;
  activeContinuationTitle?: string;
  continuationPhase?: string;
  continuationError?: string | null;
  continuationNoNewEvents?: boolean;
  liveEvents?: readonly LiveTranscriptEvent[];
  tailPartial?: boolean;
  tailDiagnostics?: number;
  tailError?: string | null;
  branchLoading?: boolean;
  branchError?: string | null;
  scanning?: boolean;
}>();
const emit = defineEmits<{ continue: []; fork: []; selectBranch: [branchId: string] }>();
const viewMode = shallowRef<'focus' | 'full'>('focus');
const searchOpen = shallowRef(false);
const searchInput = shallowRef<HTMLInputElement | null>(null);
const copyStatus = shallowRef<'copied' | 'error' | null>(null);
const turns = computed(() => (props.detail ? buildConversationTurns(props.detail) : []));
const hasTurns = computed(() => turns.value.length > 0);
const turnInsights = computed(() => props.detail?.turn_insights ?? []);
const canContinue = computed(() => props.detail?.summary.provider_id === 'claude' && Boolean(props.detail.summary.native_session_id && props.detail.summary.cwd));
const continuationActive = computed(() => Boolean(props.activeContinuationSessionId));
const continuationOnCurrentSession = computed(() => continuationActive.value && props.activeContinuationSessionId === props.detail?.summary.id);
const selectedBranch = computed(() => props.detail?.branches?.find((branch) => branch.id === props.detail?.selected_branch_id) ?? props.detail?.branches?.find((branch) => branch.id === props.detail?.active_branch_id) ?? props.detail?.branches?.find((branch) => branch.is_active));
const alternateReadOnly = computed(() => Boolean(selectedBranch.value && !selectedBranch.value.is_active && selectedBranch.value.kind !== 'active'));
const scanBlocked = computed(() => Boolean(props.scanning));
const branchSelectionBlocked = computed(() => Boolean(
  props.detail
    && props.detail.selected_branch_id
    && props.detail.active_branch_id
    && props.detail.selected_branch_id !== props.detail.active_branch_id,
));
const continueDisabled = computed(() => !canContinue.value || continuationActive.value || scanBlocked.value || Boolean(props.branchLoading) || alternateReadOnly.value || branchSelectionBlocked.value);
const continueReason = computed(() => scanBlocked.value
  ? '扫描进行中'
  : props.branchLoading
    ? '正在读取分支，完成后才能继续对话'
    : continuationOnCurrentSession.value
      ? '当前会话续聊中'
      : continuationActive.value
        ? '另一会话续聊中'
        : alternateReadOnly.value || branchSelectionBlocked.value
          ? '当前为 alternate/非主分支，仅支持只读历史浏览'
          : !canContinue.value
            ? '当前会话不支持 Claude 原生续聊'
            : '');
const currentHeadBranch = computed(() => {
  const branch = selectedBranch.value;
  return Boolean(
    branch
      && props.detail?.selected_branch_id
      && props.detail.selected_branch_id === props.detail.active_branch_id
      && branch.id === props.detail.active_branch_id
      && branch.is_active,
  );
});
const forkDisabled = computed(() => !canContinue.value || !currentHeadBranch.value || continuationActive.value || scanBlocked.value || Boolean(props.branchLoading));
const forkReason = computed(() => scanBlocked.value
  ? '扫描进行中，完成后才能分叉'
    : props.branchLoading
    ? '正在读取分支，完成后才能分叉'
    : continuationActive.value
      ? '续聊进行中，完成后才能分叉'
      : !currentHeadBranch.value
        ? '公共 Claude CLI 仅支持从当前主分支头分叉；alternate 分支仅支持只读历史浏览'
        : !canContinue.value
          ? '当前会话不支持 Claude 原生分叉'
          : '');
const continuationTitle = computed(() => continuationOnCurrentSession.value ? props.detail?.summary.title ?? '当前会话' : props.activeContinuationTitle || '另一会话');
const continuationPhaseLabel = computed(() => ({
  preflighting: '预检中',
  starting: '启动中',
  running: '运行中',
  draining: '收尾中',
  exited: '已结束',
  error: '错误',
  closed: '已关闭',
}[props.continuationPhase ?? ''] ?? '续聊进行中'));
const liveEvents = computed(() => props.liveEvents ?? []);
const sessionIdentifier = computed(() => props.detail?.summary.native_session_id ?? props.detail?.summary.id ?? null);
const readerSearch = useReaderSearch({
  detail: computed(() => props.detail),
  mode: viewMode,
  onNeedsFull: () => { viewMode.value = 'full'; },
});
const searchQuery = readerSearch.query;
const searchCurrentIndex = readerSearch.currentIndex;
const searchMatchCount = readerSearch.matchCount;
const searchFullMatchCount = readerSearch.fullMatchCount;
const searchStatus = readerSearch.status;
const searchFullOnly = readerSearch.hasFullOnlyMatch;
const keyboard = useReaderKeyboard({
  input: searchInput,
  onOpen: () => { searchOpen.value = true; },
  onNext: () => readerSearch.next(),
  onPrevious: () => readerSearch.previous(),
  onClose: () => { searchOpen.value = false; readerSearch.clear(); },
});
function closeSearch() {
  searchOpen.value = false;
  readerSearch.clear();
}
let copyStatusTimer: number | undefined;

function date(value: number | null): string {
  return value ? new Date(value).toLocaleString() : 'Time unavailable';
}

function setCopyStatus(status: 'copied' | 'error') {
  copyStatus.value = status;
  if (copyStatusTimer !== undefined) window.clearTimeout(copyStatusTimer);
  copyStatusTimer = window.setTimeout(() => { copyStatus.value = null; }, 1600);
}

async function copySessionIdentifier() {
  const value = sessionIdentifier.value;
  if (!value) return;
  setCopyStatus((await copyText(value)) ? 'copied' : 'error');
}

function insightForTurn(turnId: string): TurnInsight | null {
  const numericId = Number.parseInt(turnId.replace(/^turn-/, ''), 10);
  if (!Number.isFinite(numericId)) return null;
  return turnInsights.value.find((insight) => insight.turn_id === numericId) ?? null;
}

onScopeDispose(() => { if (copyStatusTimer !== undefined) window.clearTimeout(copyStatusTimer); });
</script>

<template>
  <main class="reader">
    <div v-if="loading" class="reader-state" role="status">Loading context…</div>
    <div v-else-if="error" class="reader-state error" role="alert">{{ error }}</div>
    <div v-else-if="!detail" class="reader-state">
      <span class="empty-glyph" aria-hidden="true">✦</span>
      <h2>Choose a session</h2>
      <p>Search or select a session to open its context spine.</p>
    </div>

    <template v-else>
      <header class="reader-head">
        <div>
          <div class="eyebrow">
            {{ detail.summary.provider_id }} <span>／</span> {{ detail.summary.project_id }}
            <span>／</span> {{ date(detail.summary.started_at) }}
          </div>
          <h1>{{ detail.summary.title }}</h1>
          <p class="reader-sub">
            {{ detail.summary.cwd || 'Working directory unavailable' }}
            <span v-if="detail.summary.branch">· {{ detail.summary.branch }}</span>
          </p>
          <div v-if="continuationActive" class="continuation-context" role="status" aria-live="polite">
            <strong>{{ continuationTitle }}</strong>
            <span>{{ continuationPhaseLabel }}</span>
          </div>
          <div class="reader-facts">
            <span>{{ detail.summary.models.join(', ') || 'Model unavailable' }}</span>
            <span>{{ detail.summary.tool_count }} tool calls</span>
            <span>Updated {{ date(detail.summary.ended_at ?? detail.summary.source_mtime) }}</span>
            <span v-if="sessionIdentifier" class="session-identifier">
              <span style="margin-left: -5px;">Session ID</span>
              <code>{{ sessionIdentifier }}</code>
              <button type="button" :aria-label="`Copy session ID ${sessionIdentifier}`" @click="copySessionIdentifier">{{ copyStatus === 'copied' ? '已复制' : copyStatus === 'error' ? '复制失败' : '复制' }}</button>
            </span>
          </div>
        </div>
        <div class="reader-actions">
          <div v-if="canContinue" class="continue-control">
            <button type="button" class="continue-button" :disabled="continueDisabled" :aria-describedby="continueReason ? 'continue-disabled-reason' : undefined" @click="emit('continue')">继续对话</button>
            <span v-if="continueReason" id="continue-disabled-reason" class="disabled-reason" role="status">{{ continueReason }}</span>
          </div>
          <div class="reader-mode" role="group" aria-label="Reader mode">
            <button type="button" :class="{ active: viewMode === 'focus' }" :aria-pressed="viewMode === 'focus'" @click="viewMode = 'focus'">Focus</button>
            <button type="button" :class="{ active: viewMode === 'full' }" :aria-pressed="viewMode === 'full'" @click="viewMode = 'full'">Full</button>
          </div>
          <button type="button" class="session-search-toggle" :aria-expanded="searchOpen" @click="searchOpen = !searchOpen">⌕ Search</button>
          <span v-if="detail.summary.partial" class="status-pill">Partial parse</span>
        </div>
      </header>

      <div v-if="searchOpen" class="reader-search-row">
        <SessionSearchControls
          :query="searchQuery"
          :current-index="searchCurrentIndex"
          :match-count="searchFullOnly ? searchFullMatchCount : searchMatchCount"
          :status="searchStatus"
          :full-only="searchFullOnly"
          @update:query="readerSearch.setQuery"
          @next="readerSearch.next"
          @previous="readerSearch.previous"
          @close="closeSearch"
          @keydown="keyboard.onSearchKeydown"
        />
      </div>

      <BranchSelector
        :branches="detail.branches ?? []"
        :selected-branch-id="detail.selected_branch_id"
        :active-branch-id="detail.active_branch_id"
        :loading="Boolean(branchLoading || scanning)"
        :error="branchError"
        :fork-disabled="forkDisabled"
        :fork-disabled-reason="forkReason"
        :scanning="scanning"
        @select="emit('selectBranch', $event)"
        @fork="emit('fork')"
      />
      <HistoryHint :detail="detail" />

      <ToolSummary v-if="viewMode === 'full'" :stats="detail.tool_stats" />

      <LiveTranscriptPanel
        :session-id="detail.summary.id"
        :active-session-id="activeContinuationSessionId ?? null"
        :continuation-phase="continuationPhase"
        :events="liveEvents"
        :no-new-events="continuationNoNewEvents ?? false"
        :tail-partial="tailPartial ?? false"
        :tail-diagnostics="tailDiagnostics ?? 0"
        :tail-error="tailError ?? null"
        :continuation-error="continuationError ?? null"
      />

      <div v-if="detail.diagnostics.length" class="partial-banner" role="status">
        Some lines could not be parsed; all readable events are shown.
      </div>

      <div class="timeline">
        <template v-if="hasTurns">
          <section v-for="turn in turns" :key="turn.id" class="conversation-turn">
            <template v-if="viewMode === 'full'">
              <TimelineEvent v-for="event in turn.orderedEvents" :key="event.id" :event="event" :query="searchQuery" :final-response="event.final_response === true" />
              <TurnChangeSummary :changes="insightForTurn(turn.id)?.file_changes" />
              <ToolSummary :stats="insightForTurn(turn.id)?.tool_stats" />
            </template>
            <template v-else>
              <TimelineEvent v-if="turn.user" :event="turn.user" :query="searchQuery" />
              <details v-if="turn.focusActivities.length" class="turn-activity">
                <summary>{{ turn.focusActivities.length }} activity event<span v-if="turn.focusActivities.length !== 1">s</span></summary>
                <TimelineEvent v-for="event in turn.focusActivities" :key="event.id" :event="event" :query="searchQuery" :final-response="false" />
              </details>
              <TimelineEvent v-if="turn.finalAssistant" :event="turn.finalAssistant" :query="searchQuery" :final-response="true" />
              <TurnChangeSummary :changes="insightForTurn(turn.id)?.file_changes" />
              <ToolSummary :stats="insightForTurn(turn.id)?.tool_stats" />
            </template>
          </section>
        </template>
        <template v-else>
          <TimelineEvent v-for="event in detail.timeline" :key="event.id" :event="event" :query="searchQuery" />
        </template>
        <div v-if="!detail.timeline.length && !turns.length" class="reader-state">
          No readable events in this session.
        </div>
      </div>
    </template>
  </main>
</template>

<style scoped>
.reader-actions { display: flex; flex-direction: column; align-items: flex-end; gap: 12px; }
.continuation-context { display: inline-flex; align-items: center; gap: 8px; margin-top: 8px; color: #c7a86c; font-size: 11px; }
.continuation-context span { color: #8fa39e; }
.continue-control { display: flex; flex-direction: column; align-items: flex-end; gap: 5px; }
.continue-button { border: 1px solid #75b9a4; border-radius: 5px; padding: 7px 10px; background: #75b9a41c; color: #c9e8dd; cursor: pointer; font: inherit; font-size: 11px; }
.continue-button:hover, .continue-button:focus-visible { outline: 0; background: #2a3d40; }
.continue-button:disabled { border-color: #4a5c59; background: transparent; color: #7f9290; cursor: not-allowed; opacity: .8; }
.disabled-reason { color: #c7a86c; font-size: 10px; }
.reader-mode { display: inline-flex; gap: 2px; padding: 2px; background: #1d272d; border-radius: 5px; }
.reader-mode button { border: 0; background: transparent; color: #829198; border-radius: 4px; padding: 5px 9px; font: inherit; font-size: 10px; cursor: pointer; }
.reader-mode button.active, .reader-mode button:hover, .reader-mode button:focus-visible { color: #c6e2d8; background: #2a3d40; outline: 0; }
.conversation-turn { margin-bottom: 10px; }
.turn-activity { margin-left: 28px; margin-bottom: 4px; border-left: 1px solid #2d3941; padding-left: 12px; }
.turn-activity summary { color: #75858d; cursor: pointer; font-size: 11px; padding: 7px 0 10px; }
.session-identifier { display: inline-flex; align-items: center; gap: 5px; min-width: 0; }
.session-identifier code { overflow: hidden; max-width: min(36vw, 330px); color: #8fa39e; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; text-overflow: ellipsis; white-space: nowrap; }
.session-identifier button { flex: none; border: 1px solid #35464d; border-radius: 4px; padding: 2px 5px; background: transparent; color: #9db0b3; cursor: pointer; font: inherit; font-size: 10px; }
.session-identifier button:hover, .session-identifier button:focus-visible { border-color: #75b9a4; color: #c9e8dd; outline: 0; }
</style>
