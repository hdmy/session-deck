<script setup lang="ts">
import { computed, onScopeDispose, shallowRef } from 'vue';
import type { KnowledgeCard, KnowledgeCardPatch, LiveTranscriptEvent, ProviderCapabilities, RelatedSession, SessionDetail, TurnInsight } from '../../types';
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
import KnowledgeCardPanel from '../knowledge/KnowledgeCardPanel.vue';
import RelatedSessionsPanel from '../knowledge/RelatedSessionsPanel.vue';
import SemanticSearchPanel from '../knowledge/SemanticSearchPanel.vue';
import { formatDateTime, useI18n } from '../../i18n';

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
  providerCapabilities?: ProviderCapabilities | null;
  knowledgeCard?: KnowledgeCard | null;
  knowledgeLoading?: boolean;
  knowledgeSaving?: boolean;
  knowledgeError?: string | null;
  relatedSessions?: readonly RelatedSession[];
  relatedLoading?: boolean;
  relatedError?: string | null;
  semanticQuery?: string;
  semanticScope?: 'all' | 'current';
  semanticResults?: readonly RelatedSession[];
  semanticLoading?: boolean;
  semanticError?: string | null;
}>();
const { t } = useI18n();
const emit = defineEmits<{
  continue: [];
  fork: [];
  selectBranch: [branchId: string];
  saveKnowledge: [patch: KnowledgeCardPatch];
  openRelatedSession: [sessionId: string];
  'updateSemanticQuery': [value: string];
  'updateSemanticScope': [value: 'all' | 'current'];
  semanticSearch: [];
}>();
const viewMode = shallowRef<'focus' | 'full'>('focus');
const searchOpen = shallowRef(false);
const searchInput = shallowRef<HTMLInputElement | null>(null);
const copyStatus = shallowRef<'copied' | 'error' | null>(null);
const turns = computed(() => (props.detail ? buildConversationTurns(props.detail) : []));
const hasTurns = computed(() => turns.value.length > 0);
const turnInsights = computed(() => props.detail?.turn_insights ?? []);
const capabilities = computed(() => props.providerCapabilities ?? {
  supports_resume: false,
  supports_branching: false,
});
const canContinue = computed(() => Boolean(capabilities.value.supports_resume && props.detail?.summary.native_session_id && props.detail.summary.cwd));
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
  ? t('continueReasonScanning')
  : props.branchLoading
    ? t('continueReasonBranchLoading')
    : continuationOnCurrentSession.value
      ? t('continueReasonCurrent')
      : continuationActive.value
        ? t('continueReasonOther')
        : alternateReadOnly.value || branchSelectionBlocked.value
          ? t('continueReasonAlternate')
          : !capabilities.value.supports_resume
            ? t('continueReasonAgent')
            : !canContinue.value
              ? t('continueReasonNative')
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
const forkDisabled = computed(() => !capabilities.value.supports_branching || !canContinue.value || !currentHeadBranch.value || continuationActive.value || scanBlocked.value || Boolean(props.branchLoading));
const forkReason = computed(() => scanBlocked.value
  ? t('forkReasonScanning')
    : props.branchLoading
    ? t('forkReasonBranchLoading')
    : continuationActive.value
      ? t('forkReasonContinuation')
      : !currentHeadBranch.value
        ? t('forkReasonHead')
        : !capabilities.value.supports_branching
          ? t('forkReasonAgent')
          : !canContinue.value
            ? t('forkReasonNative')
            : '');
const continuationTitle = computed(() => continuationOnCurrentSession.value ? props.detail?.summary.title ?? t('currentSession') : props.activeContinuationTitle || t('anotherSession'));
const continuationPhaseLabel = computed(() => ({
  preflighting: t('preflighting'),
  starting: t('starting'),
  running: t('running'),
  draining: t('draining'),
  exited: t('exited'),
  error: t('error'),
  closed: t('closed'),
}[props.continuationPhase ?? ''] ?? t('continuing')));
const liveEvents = computed(() => props.liveEvents ?? []);
const sessionIdentifier = computed(() => props.detail?.summary.native_session_id ?? props.detail?.summary.id ?? null);
const hasKnowledgeCard = computed(() => {
  const card = props.knowledgeCard;
  if (!card) return false;
  const summary = card.summary.trim();
  return Boolean(
    (summary && summary !== card.title.trim())
    || card.decisions.length
    || card.troubleshooting.length
    || card.change_summary.trim()
    || (!card.auto_generated && card.body_markdown.trim()),
  );
});
const hasRelatedSessions = computed(() => Boolean(props.relatedSessions?.length));
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
  return value ? formatDateTime(value) : t('timeUnavailable');
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
    <div v-if="loading" class="reader-state" role="status">{{ t('loadingContext') }}</div>
    <div v-else-if="error" class="reader-state error" role="alert">{{ error }}</div>
    <div v-else-if="!detail" class="reader-state">
      <span class="empty-glyph" aria-hidden="true">✦</span>
      <h2>{{ t('chooseSession') }}</h2>
      <p>{{ t('openContextSpine') }}</p>
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
            {{ detail.summary.cwd || t('workingDirectoryUnavailable') }}
            <span v-if="detail.summary.branch">· {{ detail.summary.branch }}</span>
          </p>
          <div v-if="continuationActive" class="continuation-context" role="status" aria-live="polite">
            <strong>{{ continuationTitle }}</strong>
            <span>{{ continuationPhaseLabel }}</span>
          </div>
          <div class="reader-facts">
            <div class="fact-main">
              <span>{{ detail.summary.models.join(', ') || t('modelUnavailable') }}</span>
              <span class="fact-sep" aria-hidden="true">·</span>
              <span>{{ t('toolCalls', { count: detail.summary.tool_count }) }}</span>
              <span class="fact-sep" aria-hidden="true">·</span>
              <span>{{ t('updated', { value: date(detail.summary.ended_at ?? detail.summary.source_mtime) }) }}</span>
            </div>
            <div v-if="sessionIdentifier" class="session-identifier">
              <span class="session-label">{{ t('sessionId') }}</span>
              <code>{{ sessionIdentifier }}</code>
              <button type="button" :aria-label="t('copySessionId', { id: sessionIdentifier })" @click="copySessionIdentifier">{{ copyStatus === 'copied' ? t('copied') : copyStatus === 'error' ? t('copyFailed') : t('copy') }}</button>
            </div>
          </div>
        </div>
        <div class="reader-actions">
          <div class="action-row main-actions">
            <button type="button" class="session-search-toggle" :aria-expanded="searchOpen" @click="searchOpen = !searchOpen">⌕ {{ t('search') }}</button>
            <div class="continue-control">
              <button type="button" class="continue-button" :disabled="continueDisabled" :aria-describedby="continueReason ? 'continue-disabled-reason' : undefined" @click="emit('continue')">{{ t('continueConversation') }}</button>
              <span v-if="continueReason" id="continue-disabled-reason" class="disabled-reason" role="status">{{ continueReason }}</span>
            </div>
          </div>
          <div class="action-row sub-actions">
            <div class="reader-mode" role="group" :aria-label="t('readerMode')">
              <button type="button" :class="{ active: viewMode === 'focus' }" :aria-pressed="viewMode === 'focus'" @click="viewMode = 'focus'">{{ t('focus') }}</button>
              <button type="button" :class="{ active: viewMode === 'full' }" :aria-pressed="viewMode === 'full'" @click="viewMode = 'full'">{{ t('full') }}</button>
            </div>
          </div>
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

      <details class="knowledge-reader-section">
        <summary>{{ t('historicalKnowledge') }}</summary>
        <div class="knowledge-reader-content">
          <KnowledgeCardPanel
            v-if="hasKnowledgeCard || knowledgeLoading || knowledgeError"
            :card="knowledgeCard ?? null"
            :loading="knowledgeLoading ?? false"
            :saving="knowledgeSaving ?? false"
            :error="knowledgeError ?? null"
            @save="emit('saveKnowledge', $event)"
          />
          <RelatedSessionsPanel
            v-if="hasRelatedSessions || relatedLoading || relatedError"
            :related="relatedSessions ?? []"
            :loading="relatedLoading ?? false"
            :error="relatedError ?? null"
            @open-session="emit('openRelatedSession', $event)"
          />
          <SemanticSearchPanel
            :query="semanticQuery ?? ''"
            :scope="semanticScope ?? 'all'"
            :results="semanticResults ?? []"
            :loading="semanticLoading ?? false"
            :error="semanticError ?? null"
            @update:query="emit('updateSemanticQuery', $event)"
            @update:scope="emit('updateSemanticScope', $event)"
            @search="emit('semanticSearch')"
            @open-session="emit('openRelatedSession', $event)"
          />
        </div>
      </details>

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
        {{ t('partialParseDetails') }}
      </div>

      <div class="timeline">
        <ToolSummary v-if="viewMode === 'full'" :stats="detail.tool_stats" />
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
                <summary>{{ turn.focusActivities.length === 1 ? t('activityEvent', { count: turn.focusActivities.length }) : t('activityEvents', { count: turn.focusActivities.length }) }}</summary>
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
          {{ t('noReadableEvents') }}
        </div>
      </div>
    </template>
  </main>
</template>

<style scoped>
.reader-actions {
  display: flex;
  flex: none;
  flex-direction: column;
  align-items: flex-end;
  gap: 20px;
}
.action-row {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
}
.continuation-context { display: inline-flex; align-items: center; gap: 8px; margin-top: 8px; color: #c7a86c; font-size: 11px; }
.continuation-context span { color: #8fa39e; }
.continue-control {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
}
.continue-button,
.session-search-toggle {
  flex: none;
  height: 30px;
  box-sizing: border-box;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 5px;
  padding: 0 10px;
  font: inherit;
  font-size: 11px;
  cursor: pointer;
  white-space: nowrap;
  transition: all 0.15s ease;
}
.continue-button {
  border: 1px solid #75b9a4;
  background: #75b9a41c;
  color: #c9e8dd;
}
.continue-button:hover:not(:disabled),
.continue-button:focus-visible {
  outline: 0;
  background: #2a3d40;
  border-color: #8ed6c0;
}
.continue-button:disabled {
  border-color: #4a5c59;
  background: transparent;
  color: #7f9290;
  cursor: not-allowed;
  opacity: .8;
}
.disabled-reason {
  position: absolute;
  top: 100%;
  right: 0;
  margin-top: 4px;
  color: #c7a86c;
  font-size: 10px;
  white-space: nowrap;
  text-align: right;
}
.session-search-toggle {
  border: 1px solid #34454a;
  background: #172127;
  color: #91aaa5;
}
.session-search-toggle:hover,
.session-search-toggle:focus-visible {
  border-color: #75b9a4;
  color: #c6e2d8;
  outline: 0;
}
.reader-mode {
  display: inline-flex;
  gap: 2px;
  padding: 2px;
  background: #1d272d;
  border: 1px solid #2d3b44;
  border-radius: 5px;
  height: 30px;
  box-sizing: border-box;
}
.reader-mode button {
  border: 0;
  background: transparent;
  color: #829198;
  border-radius: 4px;
  padding: 0 10px;
  font: inherit;
  font-size: 11px;
  cursor: pointer;
  height: 100%;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s ease;
}
.reader-mode button.active,
.reader-mode button:hover,
.reader-mode button:focus-visible {
  color: #c6e2d8;
  background: #2a3d40;
  outline: 0;
}
.conversation-turn { margin-bottom: 10px; }
.turn-activity { margin-left: 28px; margin-bottom: 4px; border-left: 1px solid #2d3941; padding-left: 12px; }
.turn-activity summary { color: #75858d; cursor: pointer; font-size: 11px; padding: 7px 0 10px; }
.session-identifier { display: inline-flex; align-items: center; gap: 6px; min-width: 0; }
.session-label { color: #61717d; font-size: 10px; }
.session-identifier code { overflow: hidden; max-width: min(36vw, 320px); color: #8fa39e; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 10px; text-overflow: ellipsis; white-space: nowrap; background: #172126; border: 1px solid #28353b; padding: 2px 6px; border-radius: 4px; }
.session-identifier button { flex: none; border: 1px solid #35464d; border-radius: 4px; padding: 2px 6px; background: transparent; color: #9db0b3; cursor: pointer; font: inherit; font-size: 10px; transition: border-color 0.15s ease, color 0.15s ease; }
.session-identifier button:hover, .session-identifier button:focus-visible { border-color: #75b9a4; color: #c9e8dd; outline: 0; }
.knowledge-reader-section { margin: 14px clamp(38px, 8vw, 110px) 0; border: 1px solid #2d3a42; border-radius: 6px; background: #151d23; }
.knowledge-reader-section > summary { display: flex; align-items: center; min-height: 30px; padding: 0 11px; color: #8fa39e; cursor: pointer; font-size: 11px; letter-spacing: .06em; list-style: none; }
.knowledge-reader-section > summary::-webkit-details-marker { display: none; }
.knowledge-reader-section > summary::before { width: 5px; height: 5px; margin-right: 8px; border: solid #758a87; border-width: 0 1px 1px 0; content: ''; transform: rotate(-45deg); transition: transform 120ms ease; }
.knowledge-reader-section[open] > summary { border-bottom: 1px solid #2d3a42; }
.knowledge-reader-section[open] > summary::before { transform: rotate(45deg); }
.knowledge-reader-section > summary:focus-visible { outline: 1px solid #75b9a4; outline-offset: -3px; }
.knowledge-reader-content { display: grid; gap: 14px; padding: 14px; }
</style>
