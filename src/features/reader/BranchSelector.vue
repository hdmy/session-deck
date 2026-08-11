<script setup lang="ts">
import { computed } from 'vue';
import type { BranchSummary } from '../../types';
import { useI18n } from '../../i18n';

const props = defineProps<{
  branches: readonly BranchSummary[];
  selectedBranchId?: string | null;
  activeBranchId?: string | null;
  loading?: boolean;
  error?: string | null;
  forkDisabled?: boolean;
  forkDisabledReason?: string;
  scanning?: boolean;
}>();
const emit = defineEmits<{ select: [branchId: string]; fork: [] }>();
const showSelector = computed(() => props.branches.length > 1);
const showControls = computed(() => props.branches.length > 0);
const selected = computed(() => props.selectedBranchId ?? props.activeBranchId ?? props.branches.find((branch) => branch.is_active)?.id ?? props.branches[0]?.id ?? '');
const { t } = useI18n();
function kindLabel(branch: BranchSummary): string {
  if (branch.is_active || branch.kind === 'active') return t('active');
  if (branch.compacted || branch.kind === 'compacted') return t('compacted');
  return t('alternate');
}
</script>

<template>
  <div v-if="showControls" class="branch-selector">
    <template v-if="showSelector">
      <label for="branch-select">{{ t('historyBranch') }}</label>
      <select id="branch-select" :aria-label="t('selectHistoryBranch')" :value="selected" :disabled="loading" @change="emit('select', ($event.target as HTMLSelectElement).value)">
        <option v-for="branch in props.branches" :key="branch.id" :value="branch.id">
          {{ branch.label }} · {{ kindLabel(branch) }}<template v-if="branch.compacted"> · compacted</template>
        </option>
      </select>
    </template>
    <span v-if="scanning" class="branch-status" role="status">{{ t('scanningInProgress') }}</span>
    <span v-else-if="loading" class="branch-status" role="status">{{ t('loadingBranch') }}</span>
    <span v-if="error" class="branch-status error" role="alert">{{ error }}</span>
    <button type="button" class="fork-button" :disabled="forkDisabled" :aria-describedby="forkDisabledReason ? 'fork-disabled-reason' : undefined" @click="emit('fork')">{{ t('forkFromHead') }}</button>
    <span v-if="forkDisabledReason" id="fork-disabled-reason" class="branch-status" role="status">{{ forkDisabledReason }}</span>
  </div>
</template>

<style scoped>
.fork-button { border: 1px solid #75b9a4; border-radius: 4px; padding: 4px 7px; background: transparent; color: #c9e8dd; cursor: pointer; font: inherit; font-size: 10px; }
.fork-button:disabled { border-color: #4a5c59; color: #7f9290; cursor: not-allowed; }
</style>
