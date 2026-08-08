<script setup lang="ts">
import { computed, useTemplateRef } from 'vue';

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
defineExpose({ focus: () => input.value?.focus(), input });
</script>

<template>
  <div class="session-search-controls" role="search" aria-label="Search this session">
    <input
      ref="searchInput"
      :value="props.query"
      type="search"
      aria-label="Search current session"
      placeholder="Search this session…"
      @input="emit('update:query', ($event.target as HTMLInputElement).value)"
      @keydown="emit('keydown', $event)"
    />
    <span class="session-search-count" aria-live="polite">
      <template v-if="props.error">{{ props.error }}</template>
      <template v-else-if="props.loading">Searching…</template>
      <template v-else-if="props.status === 'idle'">Type to search</template>
      <template v-else-if="props.status === 'none'">No matches</template>
      <template v-else>{{ displayCurrentIndex }} / {{ props.matchCount }}</template>
    </span>
    <span v-if="props.fullOnly" class="session-search-hint" role="status">Matches are in Full view; switch to Full to locate.</span>
    <button type="button" :disabled="!props.matchCount" aria-label="Previous match" @click="emit('previous')">↑</button>
    <button type="button" :disabled="!props.matchCount" aria-label="Next match" @click="emit('next')">↓</button>
    <button type="button" aria-label="Close session search" @click="emit('close')">×</button>
  </div>
</template>
