<script setup lang="ts">
import { computed, useTemplateRef } from 'vue';
import { useI18n } from '../../i18n';

const props = defineProps<{
  query: string;
  currentIndex: number;
  matchCount: number;
  loading?: boolean;
  status: 'idle' | 'matches' | 'none';
  fullOnly?: boolean;
  error?: string | null;
}>();
const emit = defineEmits<{
  'update:query': [value: string];
  next: [];
  previous: [];
  close: [];
  keydown: [event: KeyboardEvent];
}>();
const input = useTemplateRef<HTMLInputElement>('searchInput');
const displayCurrentIndex = computed(() => Math.max(0, props.currentIndex + 1));
const { t } = useI18n();
defineExpose({ focus: () => input.value?.focus(), input });
</script>

<template>
  <div class="session-search-controls" role="search" :aria-label="t('searchThisSession')">
    <input
      ref="searchInput"
      :value="props.query"
      type="search"
      :aria-label="t('searchCurrentSession')"
      :placeholder="t('searchThisSession') + '…'"
      @input="emit('update:query', ($event.target as HTMLInputElement).value)"
      @keydown="emit('keydown', $event)"
    />
    <span class="session-search-count" aria-live="polite">
      <template v-if="props.error">{{ props.error }}</template>
      <template v-else-if="props.loading">{{ t('searching') }}</template>
      <template v-else-if="props.status === 'idle'">{{ t('typeToSearch') }}</template>
      <template v-else-if="props.status === 'none'">{{ t('noMatches') }}</template>
      <template v-else>{{ displayCurrentIndex }} / {{ props.matchCount }}</template>
    </span>
    <span v-if="props.fullOnly" class="session-search-hint" role="status">{{ t('matchesInFullView') }}</span>
    <button type="button" :disabled="!props.matchCount" :aria-label="t('previousMatch')" @click="emit('previous')">↑</button>
    <button type="button" :disabled="!props.matchCount" :aria-label="t('nextMatch')" @click="emit('next')">↓</button>
    <button type="button" :aria-label="t('closeSessionSearch')" @click="emit('close')">×</button>
  </div>
</template>
