<script setup lang="ts">
import { computed } from 'vue';
import type { LiveTranscriptEvent } from '../../types';
import MarkdownContent from './MarkdownContent.vue';
import { formatTime, useI18n } from '../../i18n';

const props = defineProps<{
  sessionId: string;
  activeSessionId: string | null;
  continuationPhase?: string;
  noNewEvents?: boolean;
  events: readonly LiveTranscriptEvent[];
  tailPartial: boolean;
  tailDiagnostics: number;
  tailError: string | null;
  continuationError?: string | null;
}>();

const { t } = useI18n();

function label(kind: string): string {
  return ({
    user: t('you'),
    assistant: t('assistant'),
    thinking: t('thinking'),
    tool_use: t('toolCall'),
    tool_result: t('toolResult'),
    system: t('system'),
  } as Record<string, string>)[kind] ?? t('event');
}
function time(value: number | null): string {
  return formatTime(value);
}
const phaseLabel = computed(() => ({
  preflighting: t('preflighting'),
  starting: t('starting'),
  running: t('running'),
  draining: t('draining'),
  exited: t('exited'),
  error: t('error'),
  closed: t('closed'),
}[props.continuationPhase ?? ''] ?? t('continuing')));
const emptyText = computed(() => {
  if (props.continuationPhase === 'preflighting') return t('checkingClaude');
  if (props.continuationPhase === 'starting') return t('startingClaude');
  if (props.continuationPhase === 'draining') return t('drainingTranscript');
  if (props.continuationPhase === 'error') return t('continuationFailedNoEvents');
  if (props.continuationPhase === 'exited' || props.continuationPhase === 'closed') return t('continuationEndedNoEvents');
  return t('waitingForOutput');
});
const noNewEvents = computed(() => props.noNewEvents || props.events.length === 0);
</script>

<template>
  <section v-if="activeSessionId === sessionId" class="live-transcript" :aria-label="t('liveTranscript')">
    <header class="live-head">
      <div>
        <span class="eyebrow">{{ t('liveTranscript') }}</span>
        <h2>{{ t('liveContinuation') }}</h2>
      </div>
      <div class="live-head-status">
        <span v-if="props.continuationPhase" class="live-phase" role="status" aria-live="polite">{{ phaseLabel }}</span>
      <span class="live-count" role="status" aria-live="polite">{{ t('events', { count: props.events.length }) }}<span v-if="noNewEvents"> · {{ t('noNewEvents') }}</span></span>
      </div>
    </header>

    <div v-if="continuationError" class="live-banner live-error" role="alert">{{ t('continuationError', { value: continuationError }) }}</div>
    <div v-if="tailError" class="live-banner live-error" role="alert">{{ t('liveTextUnavailable', { value: tailError }) }}</div>
    <div v-if="tailPartial || tailDiagnostics" class="live-banner" role="status">
      {{ t('liveTextPartial', { suffix: tailPartial ? t('partialSuffix') : '' }) }}<span v-if="tailDiagnostics"> · {{ t('diagnostics', { count: tailDiagnostics }) }}</span>
    </div>

    <div v-if="!events.length" class="live-empty" role="status">{{ emptyText }}</div>
    <div v-else class="live-events">
      <article v-for="event in props.events" :key="String(event.id)" class="live-event" :class="`live-kind-${event.kind}`">
        <template v-if="event.kind === 'user' || event.kind === 'assistant'">
          <header class="live-event-head">
            <span>{{ label(event.kind) }}</span>
            <span v-if="event.tool_name" class="tool-name">{{ event.tool_name }}</span>
            <time v-if="event.timestamp">{{ time(event.timestamp) }}</time>
          </header>
          <MarkdownContent v-if="event.content" :content="event.content" />
        </template>
        <details v-else :open="!event.collapsed" class="live-detail">
          <summary>
            <span class="collapsed-event-disclosure" aria-hidden="true"></span>
            <span>{{ label(event.kind) }}</span>
            <span v-if="event.tool_name" class="tool-name">{{ event.tool_name }}</span>
            <time v-if="event.timestamp">{{ time(event.timestamp) }}</time>
          </summary>
          <pre v-if="event.content" class="live-content">{{ event.content }}</pre>
        </details>
      </article>
    </div>
  </section>
</template>

<style scoped>
.live-transcript { margin: 22px clamp(38px, 8vw, 110px) 0; border: 1px solid #30483f; border-radius: 8px; padding: 16px 18px 4px; background: #16231f; }
.live-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
.live-head-status { display: inline-flex; align-items: center; gap: 8px; }
.live-head h2 { margin: 4px 0 0; color: #dcefe8; font-size: 15px; }
.live-phase { color: #c9e8dd; font-size: 10px; }
.live-count { color: #82aa9c; font-size: 10px; }
.live-banner { margin: 8px 0; border-left: 2px solid #b9945d; padding: 7px 9px; background: #6b59351c; color: #d6bc89; font-size: 11px; }
.live-error { border-color: #b96f6f; background: #6b35351c; color: #e3aaaa; }
.live-empty { padding: 16px 0; color: #8ca19c; font-size: 12px; }
.live-event { border-top: 1px solid #294038; padding: 12px 0 4px; }
.live-event-head, .live-detail summary { display: flex; align-items: center; gap: 9px; margin-bottom: 7px; color: #9ec0b4; font-size: 11px; }
.live-event-head time, .live-detail summary time { margin-left: auto; color: #617d75; font-size: 10px; }
.live-detail summary { cursor: pointer; list-style: none; }
.live-detail summary::-webkit-details-marker { display: none; }
.live-detail .collapsed-event-disclosure { display: inline-grid; width: 10px; height: 10px; flex: none; place-items: center; }
.live-detail .collapsed-event-disclosure::before { width: 5px; height: 5px; border: solid #74838d; border-width: 0 1px 1px 0; content: ''; transform: rotate(-45deg); }
.live-detail[open] .collapsed-event-disclosure::before { transform: rotate(45deg); }
.tool-name { padding: 3px 6px; border-radius: 3px; background: #202a32; color: #71818d; font-size: 10px; }
.live-content { overflow-x: auto; margin: 0 0 6px 18px; white-space: pre-wrap; color: #96aaa4; font: inherit; font-size: 11px; line-height: 1.55; }
</style>
