<script setup lang="ts">
import { computed, shallowRef, watch } from 'vue';
import type { KnowledgeCard, KnowledgeCardPatch } from '../../types';
import { useI18n } from '../../i18n';

const props = defineProps<{
  card: KnowledgeCard | null;
  loading: boolean;
  saving?: boolean;
  error: string | null;
}>();

const emit = defineEmits<{ save: [patch: KnowledgeCardPatch]; cancel: [] }>();
const editing = shallowRef(false);
const title = shallowRef('');
const summary = shallowRef('');
const tags = shallowRef('');
const body = shallowRef('');
const sources = shallowRef('');
const { t } = useI18n();

function resetDraft(card: KnowledgeCard | null) {
  title.value = card?.title ?? '';
  summary.value = card?.summary ?? '';
  tags.value = card?.tags.join(', ') ?? '';
  body.value = card?.body_markdown ?? '';
  sources.value = card?.source_session_ids.join('\n') ?? '';
  editing.value = false;
}

watch(() => props.card, resetDraft, { immediate: true });
const empty = computed(() => !props.loading && !props.error && !props.card);
const showSummary = computed(() => {
  const card = props.card;
  return Boolean(card && card.summary.trim() && card.summary.trim() !== card.title.trim());
});
const showManualBody = computed(() => Boolean(
  props.card && !props.card.auto_generated && props.card.body_markdown.trim(),
));

function startEdit() { if (props.card) editing.value = true; }
function cancelEdit() { resetDraft(props.card); emit('cancel'); }
function splitLines(value: string): string[] {
  return [...new Set(value.split(/[\n,]/).map((item) => item.trim()).filter(Boolean))];
}
function save() {
  emit('save', {
    title: title.value.trim(),
    summary: summary.value.trim(),
    tags: splitLines(tags.value),
    body_markdown: body.value,
    source_session_ids: splitLines(sources.value),
  });
}
</script>

<template>
  <section class="knowledge-card-panel" :aria-label="t('knowledgeCard')">
    <div v-if="loading" class="knowledge-card-state" role="status">{{ t('loadingKnowledge') }}</div>
    <div v-else-if="error" class="knowledge-card-state error" role="alert">{{ error }}</div>
    <div v-else-if="empty" class="knowledge-card-state empty">{{ t('noKnowledgeCard') }}</div>
    <template v-else-if="card">
      <header class="knowledge-card-head">
        <div class="knowledge-header-left">
          <span v-if="!editing" class="eyebrow">{{ t('sessionInsights') }}</span>
          <input v-else v-model="title" class="knowledge-input knowledge-title-input" :aria-label="t('knowledgeTitle')" :placeholder="t('knowledgeTitle')" />
        </div>
        <div class="knowledge-card-actions">
          <button v-if="!editing" type="button" class="k-btn k-btn-secondary" @click="startEdit">{{ t('edit') }}</button>
          <template v-else>
            <button type="button" class="k-btn k-btn-primary" :disabled="saving" @click="save">{{ saving ? t('saving') : t('save') }}</button>
            <button type="button" class="k-btn k-btn-secondary" :disabled="saving" @click="cancelEdit">{{ t('cancel') }}</button>
          </template>
        </div>
      </header>

      <div class="knowledge-card-content">
        <label v-if="editing" class="knowledge-field">
          <span class="field-label">{{ t('summary') }}</span>
          <textarea v-model="summary" class="knowledge-textarea" rows="3" :placeholder="t('cardSummary')" />
        </label>
        <p v-else-if="showSummary" class="knowledge-summary">{{ card.summary }}</p>

        <label v-if="editing" class="knowledge-field">
          <span class="field-label">{{ t('tags') }}</span>
          <textarea v-model="tags" class="knowledge-textarea" rows="2" :placeholder="t('tagInputHelp')" />
        </label>
        <div v-else-if="card.tags.length" class="knowledge-tags">
          <span v-for="tag in card.tags" :key="tag" class="knowledge-tag-pill">{{ tag }}</span>
        </div>

        <div v-if="!editing && card.decisions.length" class="knowledge-section">
          <span class="section-title">{{ t('decisions') }}</span>
          <ul class="knowledge-list">
            <li v-for="item in card.decisions" :key="item">{{ item }}</li>
          </ul>
        </div>

        <div v-if="!editing && card.troubleshooting.length" class="knowledge-section">
          <span class="section-title">{{ t('troubleshooting') }}</span>
          <ul class="knowledge-list">
            <li v-for="item in card.troubleshooting" :key="item">{{ item }}</li>
          </ul>
        </div>

        <div v-if="!editing && card.change_summary" class="knowledge-section">
          <span class="section-title">{{ t('changes') }}</span>
          <p class="change-text">{{ card.change_summary }}</p>
        </div>

        <label v-if="editing" class="knowledge-field">
          <span class="field-label">{{ t('sources') }}</span>
          <textarea v-model="sources" class="knowledge-textarea" rows="3" :placeholder="t('sourceInputHelp')" />
        </label>

        <label v-if="editing" class="knowledge-field">
          <span class="field-label">{{ t('body') }}</span>
          <textarea v-model="body" class="knowledge-textarea" rows="10" :placeholder="t('markdownContent')" />
        </label>
        <pre v-else-if="showManualBody" class="knowledge-body-preview">{{ card.body_markdown }}</pre>
      </div>
    </template>
  </section>
</template>

<style scoped>
.knowledge-card-panel {
  border: 1px solid #283540;
  border-radius: 8px;
  background: #161e25;
  padding: 16px 20px;
  color: #d1dedd;
  font-size: 12px;
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.25);
}

.knowledge-card-state {
  padding: 16px 0;
  color: #71808b;
  text-align: center;
}
.knowledge-card-state.error {
  color: #d88d8d;
}

.knowledge-card-head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid #26323c;
}

.knowledge-header-left {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
  flex: 1;
}

.eyebrow {
  color: #75b9a4;
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.1em;
  text-transform: uppercase;
}

.knowledge-title {
  margin: 0;
  color: #f1f4f5;
  font-size: 18px;
  font-weight: 600;
  line-height: 1.3;
}

.knowledge-card-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: none;
}

.status-pill {
  padding: 3px 7px;
  border: 1px solid #4a5c57;
  border-radius: 4px;
  background: #1e2c28;
  color: #8fc4b4;
  font-size: 10px;
}

.k-btn {
  height: 28px;
  box-sizing: border-box;
  padding: 0 10px;
  border-radius: 5px;
  font: inherit;
  font-size: 11px;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s ease;
}

.k-btn-primary {
  border: 1px solid #75b9a4;
  background: #75b9a422;
  color: #c9e8dd;
}
.k-btn-primary:hover:not(:disabled) {
  background: #2a3d40;
  border-color: #8ed6c0;
}

.k-btn-secondary {
  border: 1px solid #34454a;
  background: transparent;
  color: #91aaa5;
}
.k-btn-secondary:hover:not(:disabled) {
  border-color: #75b9a4;
  color: #c6e2d8;
}
.k-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.knowledge-card-content {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-top: 12px;
}

.knowledge-summary {
  margin: 0;
  color: #cad7d6;
  line-height: 1.5;
}

.knowledge-meta {
  display: flex;
  align-items: center;
  gap: 6px;
  color: #6a7c85;
  font-size: 10px;
}
.meta-sep {
  color: #3b4b54;
}

.knowledge-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.field-label {
  color: #7c8e96;
  font-size: 11px;
  font-weight: 500;
}

.knowledge-input,
.knowledge-textarea {
  width: 100%;
  box-sizing: border-box;
  border: 1px solid #2d3c47;
  border-radius: 6px;
  background: #121920;
  color: #e5eceb;
  padding: 6px 10px;
  font: inherit;
  font-size: 12px;
  outline: 0;
  transition: border-color 0.15s ease;
}
.knowledge-input:focus,
.knowledge-textarea:focus {
  border-color: #75b9a4;
  box-shadow: 0 0 0 2px rgba(117, 185, 164, 0.15);
}

.knowledge-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.knowledge-tag-pill {
  padding: 2px 7px;
  border: 1px solid #2f3d47;
  border-radius: 4px;
  background: #1c2630;
  color: #9cb3b6;
  font-size: 11px;
}

.knowledge-section {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.section-title {
  color: #7c8e96;
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.05em;
  text-transform: uppercase;
}

.topic-text,
.change-text,
.source-list {
  color: #b7c7c6;
  line-height: 1.45;
}

.knowledge-list {
  margin: 0;
  padding-left: 16px;
  color: #b7c7c6;

  li {
    margin-bottom: 2px;
  }
}

.knowledge-body-preview {
  margin: 4px 0 0;
  padding: 10px 12px;
  border: 1px solid #232f38;
  border-radius: 6px;
  background: #11171d;
  color: #a4b7b5;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}
</style>
