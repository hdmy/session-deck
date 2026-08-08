<script setup lang="ts">
import { computed } from 'vue';

const props = defineProps<{ text: string; query?: string }>();

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

const parts = computed(() => {
  const query = props.query?.trim();
  if (!query) return [{ value: props.text, match: false }];
  const expression = new RegExp(`(${escapeRegExp(query)})`, 'giu');
  const matcher = new RegExp(escapeRegExp(query), 'iu');
  return props.text.split(expression).filter(Boolean).map((value) => ({
    value,
    match: matcher.test(value),
  }));
});
</script>

<template>
  <template v-for="(part, index) in parts" :key="`${index}-${part.value}`">
    <mark v-if="part.match" class="reader-highlight">{{ part.value }}</mark>
    <template v-else>{{ part.value }}</template>
  </template>
</template>
