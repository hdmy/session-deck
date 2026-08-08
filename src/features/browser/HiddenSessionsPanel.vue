<script setup lang="ts">
import type { SessionSummary } from '../../types';

const props = defineProps<{
  sessions: readonly SessionSummary[];
  loading: boolean;
  error: string | null;
  busyId?: string | null;
  activeContinuationSessionId?: string | null;
  activeContinuationTargetSessionId?: string | null;
}>();

const emit = defineEmits<{ restore: [id: string] }>();

function date(value: number | null): string {
  return value ? new Date(value).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' }) : 'Unknown time';
}

function continuationLocked(sessionId: string): boolean {
  return sessionId === props.activeContinuationSessionId || sessionId === props.activeContinuationTargetSessionId;
}

function continuationReason(sessionId: string): string {
  return sessionId === props.activeContinuationTargetSessionId
    ? '该分叉子会话正在续聊，结束后才能恢复'
    : '该会话是当前续聊的父会话，结束后才能恢复';
}
</script>

<template>
  <section class="hidden-panel" aria-labelledby="hidden-sessions-heading">
    <div class="section-label hidden-heading">
      <span id="hidden-sessions-heading">隐藏</span>
      <span v-if="loading" role="status">Loading…</span>
      <span v-else>{{ sessions.length }}</span>
    </div>
    <div v-if="error" class="nav-notice error hidden-notice" role="alert">{{ error }}</div>
    <div v-else-if="loading && !sessions.length" class="empty-mini" role="status">Loading hidden sessions…</div>
    <div v-else-if="!sessions.length" class="empty-mini">暂无隐藏会话</div>
    <ul v-else class="hidden-list">
      <li v-for="session in sessions" :key="session.id" class="hidden-session">
        <div class="hidden-session-copy">
          <strong>{{ session.title }}</strong>
          <small>{{ date(session.ended_at) }} · {{ session.branch || '无分支' }}</small>
        </div>
        <button
          type="button"
          class="restore-button"
          :disabled="Boolean(busyId) || continuationLocked(session.id)"
          :aria-describedby="continuationLocked(session.id) ? `restore-lock-${session.id}` : undefined"
          @click="emit('restore', session.id)"
        >{{ busyId === session.id ? '恢复中…' : '取消隐藏' }}</button>
        <span v-if="continuationLocked(session.id)" :id="`restore-lock-${session.id}`" class="restore-disabled-reason" role="status">
          {{ continuationReason(session.id) }}
        </span>
      </li>
    </ul>
  </section>
</template>
