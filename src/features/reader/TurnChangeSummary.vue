<script setup lang="ts">
import { computed } from 'vue';
import type { FileChangeSummary, TurnInsight } from '../../types';
import { normalizeFilePath, positiveCount } from './turnSummary';

const props = defineProps<{
  changes?: readonly FileChangeSummary[] | null;
  insight?: Pick<TurnInsight, 'file_changes'> | null;
}>();

const visibleChanges = computed(() => props.changes ?? props.insight?.file_changes ?? []);

function kind(value: string): string {
  const normalized = value.trim().toLowerCase();
  return normalized || 'changed';
}
</script>

<template>
  <section v-if="visibleChanges.length" class="turn-changes" data-testid="turn-change-summary" aria-label="File changes">
    <h3>Files changed</h3>
    <ul>
      <li v-for="(change, index) in visibleChanges" :key="`${change.path}-${change.kind}-${index}`">
        <code>{{ normalizeFilePath(change.path) }}</code>
        <span class="change-kind">{{ kind(change.kind) }}</span>
        <span class="change-lines" :aria-label="`${positiveCount(change.additions)} additions, ${positiveCount(change.deletions)} deletions`">
          <span class="additions">+{{ positiveCount(change.additions) }}</span>
          <span class="deletions">−{{ positiveCount(change.deletions) }}</span>
        </span>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.turn-changes { margin: 8px 0 12px 28px; border-left: 1px solid #35454c; padding-left: 12px; }
.turn-changes h3 { margin: 0 0 6px; color: #9eafb3; font-size: 10px; font-weight: 600; letter-spacing: .04em; text-transform: uppercase; }
.turn-changes ul { display: grid; gap: 4px; margin: 0; padding: 0; list-style: none; }
.turn-changes li { display: flex; flex-wrap: wrap; align-items: baseline; gap: 7px; color: #cbd6d8; font-size: 11px; }
.turn-changes code { min-width: 0; overflow-wrap: anywhere; color: #c6e2d8; font: inherit; }
.change-kind { color: #809197; }
.change-lines { display: inline-flex; gap: 5px; font-variant-numeric: tabular-nums; }
.additions { color: #8fc9a7; }
.deletions { color: #d99b93; }
</style>
