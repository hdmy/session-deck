<script setup lang="ts">
import { computed } from 'vue';
import type { ProviderDescriptor, ProviderId } from '../../types';
import { useI18n } from '../../i18n';

const props = defineProps<{
  providers: readonly ProviderDescriptor[];
  modelValue?: ProviderId | null;
  selectedProviderId?: ProviderId | null;
  /** Alias for integrations that use providerId as their filter prop. */
  providerId?: ProviderId | null;
}>();

const emit = defineEmits<{
  'update:modelValue': [providerId: ProviderId | null];
  'update:selectedProviderId': [providerId: ProviderId | null];
  change: [providerId: ProviderId | null];
  select: [providerId: ProviderId | null];
}>();

const selected = computed(() => {
  if (props.modelValue !== undefined) return props.modelValue;
  return props.selectedProviderId ?? props.providerId ?? null;
});
const { t } = useI18n();

function onChange(event: Event) {
  const value = (event.target as HTMLSelectElement).value;
  const providerId = value ? (value as ProviderId) : null;
  emit('update:modelValue', providerId);
  emit('update:selectedProviderId', providerId);
  emit('change', providerId);
  emit('select', providerId);
}
</script>

<template>
  <label class="provider-filter">
    <span class="provider-filter-label">{{ t('agent') }}</span>
    <select :value="selected ?? ''" :aria-label="t('agent')" @change="onChange">
      <option value="">{{ t('allAgents') }}</option>
      <option v-for="provider in providers" :key="provider.provider_id" :value="provider.provider_id">
        {{ provider.alias || provider.name }}
      </option>
    </select>
  </label>
</template>

<style scoped>
.provider-filter {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0 16px 14px;
  padding: 0;
  color: #9baeb1;
  font-size: 11px;
}
.provider-filter-label {
  color: #71868a;
  font-size: 11px;
  font-weight: 500;
}
.provider-filter select {
  min-width: 0;
  flex: 1;
  height: 28px;
  border: 1px solid #2c3b42;
  border-radius: 6px;
  background: #172127;
  color: #d1dedd;
  padding: 0 8px;
  font: inherit;
  font-size: 12px;
  outline: 0;
  transition: border-color 0.15s ease;
}
.provider-filter select:focus-visible,
.provider-filter select:hover {
  border-color: #75b9a4;
}
</style>
