<script setup lang="ts">
import type { RelatedSession } from '../../types';
import { useI18n } from '../../i18n';

const props = defineProps<{
  query: string;
  scope: 'all' | 'current';
  loading: boolean;
  error: string | null;
  results: readonly RelatedSession[];
}>();

const emit = defineEmits<{
  'update:query': [value: string];
  'update:scope': [value: 'all' | 'current'];
  search: [];
  openSession: [sessionId: string];
}>();
const { t } = useI18n();

function onScopeChange(event: Event) {
  emit('update:scope', (event.target as HTMLSelectElement).value as 'all' | 'current');
}
function onQueryInput(event: Event) {
  emit('update:query', (event.target as HTMLInputElement).value);
}
</script>

<template>
  <section class="semantic-search-panel" :aria-label="t('semanticSearch')">
    <h3 class="panel-title">{{ t('semanticSearch') }}</h3>
    <form class="semantic-search-form" @submit.prevent="emit('search')">
      <input
        :value="props.query"
        type="search"
        class="semantic-input"
        :aria-label="t('semanticSearchQuery')"
        :placeholder="t('findRelatedKnowledge')"
        @input="onQueryInput"
      />
      <select :value="props.scope" class="semantic-select" :aria-label="t('semanticSearchScope')" @change="onScopeChange">
        <option value="all">{{ t('allAgents') }}</option>
        <option value="current">{{ t('currentAgent') }}</option>
      </select>
      <button type="submit" class="semantic-btn" :disabled="loading">{{ loading ? t('searching') : t('search') }}</button>
    </form>
    <div v-if="loading" class="panel-state" role="status">{{ t('searchingKnowledge') }}</div>
    <div v-else-if="error" class="panel-state error" role="alert">{{ error }}</div>
    <div v-else-if="!props.query.trim()" class="panel-state empty">{{ t('enterKeyword') }}</div>
    <div v-else-if="!results.length" class="panel-state empty">{{ t('noSemanticMatches') }}</div>
    <ul v-else class="semantic-results">
      <li v-for="item in results" :key="`${item.session.provider_id}:${item.session.id}`">
        <button type="button" class="semantic-result" @click="emit('openSession', item.session.id)">
          <div class="result-head">
            <strong class="result-title">{{ item.session.title }}</strong>
            <span class="score-badge">{{ t('score') }} {{ item.score.toFixed(2) }}</span>
          </div>
          <p v-if="item.summary || item.reason" class="result-desc">{{ item.summary || item.reason }}</p>
          <div v-if="item.topics?.length || item.tags?.length" class="result-meta">
            <small v-if="item.topics?.length">{{ t('topics') }}: {{ item.topics.join(' · ') }}</small>
            <small v-if="item.tags?.length">{{ t('tags') }}: {{ item.tags.join(' · ') }}</small>
          </div>
        </button>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.semantic-search-panel {
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

.semantic-search-form {
  display: flex;
  align-items: center;
  gap: 8px;
}

.semantic-input {
  min-width: 0;
  flex: 1;
  height: 30px;
  box-sizing: border-box;
  border: 1px solid #2d3c47;
  border-radius: 6px;
  background: #11171d;
  color: #e5eceb;
  padding: 0 10px;
  font: inherit;
  font-size: 12px;
  outline: 0;
  transition: border-color 0.15s ease;
}
.semantic-input:focus {
  border-color: #75b9a4;
  box-shadow: 0 0 0 2px rgba(117, 185, 164, 0.15);
}

.semantic-select {
  height: 30px;
  box-sizing: border-box;
  border: 1px solid #2d3c47;
  border-radius: 6px;
  background: #11171d;
  color: #c5d3d2;
  padding: 0 8px;
  font: inherit;
  font-size: 11px;
  outline: 0;
  cursor: pointer;
  transition: border-color 0.15s ease;
}
.semantic-select:focus,
.semantic-select:hover {
  border-color: #75b9a4;
}

.semantic-btn {
  height: 30px;
  box-sizing: border-box;
  padding: 0 12px;
  border: 1px solid #75b9a4;
  border-radius: 6px;
  background: #75b9a422;
  color: #c9e8dd;
  font: inherit;
  font-size: 11px;
  cursor: pointer;
  white-space: nowrap;
  transition: all 0.15s ease;
}
.semantic-btn:hover:not(:disabled) {
  background: #2a3d40;
  border-color: #8ed6c0;
}
.semantic-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.panel-state {
  padding: 10px 0;
  color: #6a7c85;
  font-size: 11px;
}
.panel-state.error {
  color: #d88d8d;
}

.semantic-results {
  margin: 10px 0 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.semantic-result {
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

.semantic-result:hover,
.semantic-result:focus-visible {
  border-color: #75b9a4;
  background: #19232a;
  outline: 0;
}

.result-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
}

.result-title {
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

.result-desc {
  margin: 0;
  color: #a2b4b3;
  font-size: 11px;
  line-height: 1.45;
}

.result-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  color: #6c7e87;
  font-size: 10px;
}
</style>
