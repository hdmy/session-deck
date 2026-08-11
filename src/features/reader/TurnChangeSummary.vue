<script setup lang="ts">
import { computed } from 'vue';
import type { FileChangeSummary, TurnInsight } from '../../types';
import { normalizeFilePath, positiveCount } from './turnSummary';
import { useI18n } from '../../i18n';

const props = defineProps<{
  changes?: readonly FileChangeSummary[] | null;
  insight?: Pick<TurnInsight, 'file_changes'> | null;
}>();

const visibleChanges = computed(() => props.changes ?? props.insight?.file_changes ?? []);
const { t } = useI18n();

function kind(value: string): string {
  const normalized = value.trim().toLowerCase();
  return normalized || 'changed';
}
</script>

<template>
  <section v-if="visibleChanges.length" class="turn-changes" data-testid="turn-change-summary" :aria-label="t('filesChanged')">
    <h3>{{ t('filesChanged') }}</h3>
    <ul>
      <li v-for="(change, index) in visibleChanges" :key="`${change.path}-${change.kind}-${index}`">
        <code>{{ normalizeFilePath(change.path) }}</code>
        <span class="change-kind">{{ kind(change.kind) }}</span>
        <span class="change-lines" :aria-label="`${positiveCount(change.additions)} ${t('additions')}, ${positiveCount(change.deletions)} ${t('deletions')}`">
          <span class="additions">+{{ positiveCount(change.additions) }}</span>
          <span class="deletions">−{{ positiveCount(change.deletions) }}</span>
        </span>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.turn-changes { margin: 16px 0 24px 28px; border-left: none; padding-left: 0; }
.turn-changes h3 { margin: 0 0 6px; color: #9eafb3; font-size: 10px; font-weight: 600; letter-spacing: .04em; text-transform: uppercase; }
.turn-changes ul { display: grid; gap: 4px; margin: 0; padding: 0; list-style: none; }
.turn-changes li { display: flex; flex-wrap: wrap; align-items: baseline; gap: 7px; color: #cbd6d8; font-size: 11px; }
.turn-changes code { min-width: 0; overflow-wrap: anywhere; color: #c6e2d8; font: inherit; }
.change-kind { color: #809197; }
.change-lines { display: inline-flex; gap: 5px; font-variant-numeric: tabular-nums; }
.additions { color: #8fc9a7; }
.deletions { color: #d99b93; }
</style>
