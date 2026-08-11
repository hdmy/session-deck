<script setup lang="ts">
import { shallowRef } from 'vue';
import type { SessionSummary } from '../../types';
import { formatDateTime, useI18n } from '../../i18n';

const props = defineProps<{
  sessions: readonly SessionSummary[];
  selectedId?: string | null;
  loading: boolean;
  error: string | null;
  busyId?: string | null;
  activeContinuationSessionId?: string | null;
  activeContinuationTargetSessionId?: string | null;
}>();

const emit = defineEmits<{
  select: [id: string];
  restore: [id: string];
}>();

const open = shallowRef(false);
const { t } = useI18n();

function date(value: number | null): string {
  return value ? formatDateTime(value, { dateStyle: 'medium', timeStyle: 'short' }) : t('unknownTime');
}

function continuationLocked(sessionId: string): boolean {
  return sessionId === props.activeContinuationSessionId || sessionId === props.activeContinuationTargetSessionId;
}

function continuationReason(sessionId: string): string {
  return sessionId === props.activeContinuationTargetSessionId
    ? t('forkRestoreLocked')
    : t('parentRestoreLocked');
}
</script>

<template>
  <section class="project-group hidden-panel" aria-labelledby="hidden-sessions-heading">
    <div class="project-head-row">
      <button
        class="project-head"
        type="button"
        :aria-expanded="open"
        @click="open = !open"
      >
        <span class="chevron" aria-hidden="true">{{ open ? '▾' : '▸' }}</span>
        <span id="hidden-sessions-heading" class="project-name">{{ t('hiddenSessions') }}</span>
        <span v-if="loading" class="project-count" role="status">{{ t('loading') }}</span>
        <span v-else class="project-count">{{ sessions.length }}</span>
      </button>
    </div>

    <div v-if="open" class="session-list hidden-list">
      <div v-if="error" class="nav-notice error hidden-notice" role="alert">{{ error }}</div>
      <div v-else-if="loading && !sessions.length" class="empty-mini" role="status">{{ t('loadingHiddenSessions') }}</div>
      <div v-else-if="!sessions.length" class="empty-mini">{{ t('noHiddenSessions') }}</div>
      <ul v-else class="hidden-sessions-ul">
        <li v-for="session in sessions" :key="session.id" class="session-item-row hidden-session">
          <button
            type="button"
            class="session-item"
            :aria-current="selectedId === session.id ? 'page' : undefined"
            @click="emit('select', session.id)"
          >
            <span class="session-dot" aria-hidden="true"></span>
            <span class="session-copy">
              <strong>{{ session.title }}</strong>
              <small>{{ date(session.ended_at) }} · {{ session.branch || t('noBranch') }}</small>
            </span>
          </button>
          <div class="session-actions hidden-session-actions" role="group" :aria-label="t('actionsForHidden', { title: session.title })">
            <button
              type="button"
              class="session-action restore-button"
              :disabled="Boolean(busyId) || continuationLocked(session.id)"
              :aria-describedby="continuationLocked(session.id) ? `restore-lock-${session.id}` : undefined"
              :title="t('restore')"
              :aria-label="t('restore')"
              @click.stop="emit('restore', session.id)"
            >
              <svg class="action-icon" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/>
                <path d="M3 3v5h5"/>
              </svg>
            </button>
          </div>
          <span v-if="continuationLocked(session.id)" :id="`restore-lock-${session.id}`" class="session-disabled-reason" role="status">
            {{ continuationReason(session.id) }}
          </span>
        </li>
      </ul>
    </div>
  </section>
</template>
