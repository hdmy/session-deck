<script setup lang="ts">
import type { RelatedSession } from '../../types';
import { useI18n } from '../../i18n';

defineProps<{
  related: readonly RelatedSession[];
  loading: boolean;
  error: string | null;
}>();

const emit = defineEmits<{ openSession: [sessionId: string] }>();
const { t } = useI18n();
</script>

<template>
  <section class="related-sessions-panel" :aria-label="t('relatedSessions')">
    <h3 class="panel-title">{{ t('relatedSessions') }}</h3>
    <div v-if="loading" class="panel-state" role="status">{{ t('loadingRelatedSessions') }}</div>
    <div v-else-if="error" class="panel-state error" role="alert">{{ error }}</div>
    <div v-else-if="!related.length" class="panel-state empty">{{ t('noRelatedSessions') }}</div>
    <ul v-else class="related-sessions-list">
      <li v-for="item in related" :key="`${item.relation_type}:${item.session.id}`">
        <button type="button" class="related-session" @click="emit('openSession', item.session.id)">
          <div class="session-head">
            <strong class="session-title">{{ item.session.title }}</strong>
            <span class="score-badge">{{ item.relation_type }} · {{ item.score.toFixed(2) }}</span>
          </div>
          <small v-if="item.reason" class="session-reason">{{ item.reason }}</small>
        </button>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.related-sessions-panel {
  border: 1px solid #283540;
  border-radius: 8px;
  background: #161e25;
  padding: 14px 18px;
  color: #d1dedd;
  font-size: 12px;
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.2);
}

.panel-title {
  margin: 0 0 10px;
  color: #8fa39e;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.panel-state {
  padding: 10px 0;
  color: #6a7c85;
  font-size: 11px;
}
.panel-state.error {
  color: #d88d8d;
}

.related-sessions-list {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.related-session {
  display: flex;
  flex-direction: column;
  gap: 4px;
  width: 100%;
  padding: 8px 10px;
  border: 1px solid #232f38;
  border-radius: 6px;
  background: #11171d;
  color: inherit;
  cursor: pointer;
  text-align: left;
  transition: all 0.15s ease;
}

.related-session:hover,
.related-session:focus-visible {
  border-color: #75b9a4;
  background: #19232a;
  outline: 0;
}

.session-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
}

.session-title {
  color: #e5eceb;
  font-size: 12px;
  font-weight: 550;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.score-badge {
  padding: 2px 6px;
  border-radius: 4px;
  background: #1f2d33;
  color: #75b9a4;
  font-size: 10px;
  font-weight: 500;
  white-space: nowrap;
}

.session-reason {
  color: #7c8e96;
  font-size: 10px;
  line-height: 1.4;
}
</style>
