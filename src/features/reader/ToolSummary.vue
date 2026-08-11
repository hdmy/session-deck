<script setup lang="ts">
import { computed } from 'vue';
import type { ToolStat } from '../../types';
import { useI18n } from '../../i18n';

const props = defineProps<{ stats?: readonly ToolStat[] | null }>();
const { t } = useI18n();
const visibleStats = computed(() => (props.stats ?? []).filter((stat) => stat.count > 0 || stat.successes > 0 || stat.failures > 0 || stat.unknown > 0 || stat.files_changed > 0 || stat.additions > 0 || stat.deletions > 0));
const totals = computed(() => visibleStats.value.reduce((total, stat) => ({
  count: total.count + safeNumber(stat.count),
  successes: total.successes + safeNumber(stat.successes),
  failures: total.failures + safeNumber(stat.failures),
  unknown: total.unknown + safeNumber(stat.unknown),
  files: total.files + safeNumber(stat.files_changed),
  additions: total.additions + safeNumber(stat.additions),
  deletions: total.deletions + safeNumber(stat.deletions),
}), { count: 0, successes: 0, failures: 0, unknown: 0, files: 0, additions: 0, deletions: 0 }));

function safeNumber(value: number): number {
  return Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}
</script>

<template>
  <details v-if="visibleStats.length" class="tool-summary" data-testid="tool-summary">
    <summary>
      <span>{{ t('toolSummary') }}</span>
      <span class="tool-total">{{ totals.count }} {{ t('total') }}</span>
    </summary>
    <div class="tool-overview" :aria-label="t('toolTotals')">
      <span>{{ totals.successes }} {{ t('success') }}</span>
      <span>{{ totals.failures }} {{ t('fail') }}</span>
      <span>{{ totals.unknown }} {{ t('unknown') }}</span>
      <span>{{ totals.files }} {{ t('files') }}</span>
      <span>+{{ totals.additions }} / −{{ totals.deletions }} {{ t('lines') }}</span>
    </div>
    <ul>
      <li v-for="stat in visibleStats" :key="stat.name">
        <strong>{{ stat.name || t('unknownTool') }}</strong>
        <span>{{ safeNumber(stat.count) }} {{ t('calls') }}</span>
        <span>{{ safeNumber(stat.successes) }} {{ t('success') }}</span>
        <span>{{ safeNumber(stat.failures) }} {{ t('fail') }}</span>
        <span>{{ safeNumber(stat.unknown) }} {{ t('unknown') }}</span>
        <span>{{ safeNumber(stat.files_changed) }} {{ t('files') }}</span>
        <span>+{{ safeNumber(stat.additions) }} / −{{ safeNumber(stat.deletions) }} {{ t('lines') }}</span>
      </li>
    </ul>
  </details>
</template>

<style scoped>
.tool-summary { margin: 20px 0 32px 28px; border-left: none; padding-left: 0; color: #9eafb3; font-size: 11px; }
.tool-summary summary { display: flex; align-items: center; gap: 8px; cursor: pointer; padding: 6px 0; color: #b9c9c9; outline: none; }
.tool-summary summary:focus-visible { outline: 1px solid #75b9a4; outline-offset: 3px; border-radius: 2px; }
.tool-total { color: #c6e2d8; font-variant-numeric: tabular-nums; }
.tool-overview { display: flex; flex-wrap: wrap; gap: 6px 12px; padding: 2px 0 7px; color: #84969a; font-variant-numeric: tabular-nums; }
.tool-summary ul { display: grid; gap: 5px; margin: 0; padding: 0; list-style: none; }
.tool-summary li { display: flex; flex-wrap: wrap; gap: 5px 9px; padding: 5px 0; border-top: 1px solid #29373c; font-variant-numeric: tabular-nums; }
.tool-summary li strong { color: #d2dfdc; }
</style>
