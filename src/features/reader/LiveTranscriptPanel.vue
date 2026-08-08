<script setup lang="ts">
import { computed } from 'vue';
import type { LiveTranscriptEvent } from '../../types';
import MarkdownContent from './MarkdownContent.vue';

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

const labels: Record<string, string> = {
  user: 'You',
  assistant: 'Assistant',
  thinking: 'Thinking',
  tool_use: 'Tool call',
  tool_result: 'Tool result',
  system: 'System',
};

function label(kind: string): string { return labels[kind] ?? 'Event'; }
function time(value: number | null): string {
  return value ? new Date(value).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : '';
}
const phaseLabel = computed(() => ({
  preflighting: '预检中',
  starting: '启动中',
  running: '运行中',
  draining: '收尾中',
  exited: '已结束',
  error: '错误',
  closed: '已关闭',
}[props.continuationPhase ?? ''] ?? '等待续聊'));
const emptyText = computed(() => {
  if (props.continuationPhase === 'preflighting') return '正在检查 Claude 环境…';
  if (props.continuationPhase === 'starting') return '正在启动 Claude…';
  if (props.continuationPhase === 'draining') return '进程已退出，正在收尾 transcript…';
  if (props.continuationPhase === 'error') return '续聊失败，未收到更多事件。';
  if (props.continuationPhase === 'exited' || props.continuationPhase === 'closed') return '续聊已结束，没有新事件。';
  return '暂无新事件，等待 Claude 输出…';
});
const noNewEvents = computed(() => props.noNewEvents || props.events.length === 0);
</script>

<template>
  <section v-if="activeSessionId === sessionId" class="live-transcript" aria-label="Live continuation transcript">
    <header class="live-head">
      <div>
        <span class="eyebrow">Live transcript</span>
        <h2>实时续聊</h2>
      </div>
      <div class="live-head-status">
        <span v-if="props.continuationPhase" class="live-phase" role="status" aria-live="polite">{{ phaseLabel }}</span>
      <span class="live-count" role="status" aria-live="polite">{{ props.events.length }} events<span v-if="noNewEvents"> · 暂无新事件</span></span>
      </div>
    </header>

    <div v-if="continuationError" class="live-banner live-error" role="alert">续聊错误：{{ continuationError }}</div>
    <div v-if="tailError" class="live-banner live-error" role="alert">实时文本暂不可用：{{ tailError }}</div>
    <div v-if="tailPartial || tailDiagnostics" class="live-banner" role="status">
      实时文本{{ tailPartial ? '尚未完整落盘' : '' }}<span v-if="tailDiagnostics"> · {{ tailDiagnostics }} 条诊断</span>
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
