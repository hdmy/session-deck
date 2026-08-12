<script setup lang="ts">
import type { ProviderDescriptor, ProviderId } from '../../types';
import { useI18n } from '../../i18n';

const props = defineProps<{
  intervalSeconds: number;
  providers: readonly ProviderDescriptor[];
  enabledProviderIds: readonly ProviderId[];
  providerLookbackDays: Readonly<Partial<Record<ProviderId, number>>>;
  disabled?: boolean;
  disabledReason?: string | null;
  saving?: boolean;
}>();
const emit = defineEmits<{
  'update:intervalSeconds': [value: number];
  'update:enabledProviderIds': [value: ProviderId[]];
  'update:providerLookbackDays': [value: Partial<Record<ProviderId, number>>];
  save: [];
}>();
const { t } = useI18n();
const lookbackOptions = [7, 30, 90, 180, 365] as const;

function toggleProvider(providerId: ProviderId, enabled: boolean) {
  emit(
    'update:enabledProviderIds',
    enabled
      ? [...props.enabledProviderIds, providerId]
      : props.enabledProviderIds.filter((id) => id !== providerId),
  );
}

function updateLookback(providerId: ProviderId, days: number) {
  const next = { ...props.providerLookbackDays };
  if (days === 0) delete next[providerId];
  else next[providerId] = days;
  emit('update:providerLookbackDays', next);
}
</script>

<template>
  <section class="scan-section" aria-labelledby="scan-schedule-title">
    <h3 id="scan-schedule-title">{{ t('automaticScan') }}</h3>
    <fieldset class="agent-fieldset">
      <legend>{{ t('importAgents') }}</legend>
      <div v-for="provider in props.providers" :key="provider.provider_id" class="agent-option">
        <label class="agent-toggle">
          <input
            type="checkbox"
            :value="provider.provider_id"
            :checked="props.enabledProviderIds.includes(provider.provider_id)"
            :disabled="props.disabled"
            @change="toggleProvider(provider.provider_id, ($event.target as HTMLInputElement).checked)"
          />
          <span>{{ provider.alias || provider.name }}</span>
        </label>
        <select
          class="agent-lookback"
          :data-provider-id="provider.provider_id"
          :aria-label="t('agentImportRange', { agent: provider.alias || provider.name })"
          :value="props.providerLookbackDays[provider.provider_id] ?? 0"
          :disabled="props.disabled || !props.enabledProviderIds.includes(provider.provider_id)"
          @change="updateLookback(provider.provider_id, Number(($event.target as HTMLSelectElement).value))"
        >
          <option :value="0">{{ t('allHistory') }}</option>
          <option v-for="days in lookbackOptions" :key="days" :value="days">{{ t('lastDays', { days }) }}</option>
        </select>
      </div>
    </fieldset>
    <p class="field-help">{{ t('scanAgentsHelp') }}</p>
    <label class="scan-field">{{ t('intervalSeconds') }}
      <input
        type="number"
        min="0"
        max="3600"
        step="60"
        :value="props.intervalSeconds"
        :disabled="props.disabled"
        aria-describedby="scan-interval-help scan-disabled-reason"
        @input="emit('update:intervalSeconds', Number(($event.target as HTMLInputElement).value))"
      />
    </label>
    <p id="scan-interval-help" class="field-help">{{ t('scanIntervalHelp') }}</p>
    <p v-if="props.disabled && props.disabledReason" id="scan-disabled-reason" class="field-help" role="status">{{ props.disabledReason }}</p>
    <button class="secondary-button" type="button" :disabled="props.disabled || props.saving" @click="emit('save')">{{ props.saving ? t('saving') : t('saveScanSettings') }}</button>
  </section>
</template>

<style scoped>
.scan-section { display: grid; gap: 9px; }.scan-section h3 { margin: 0; color: #b8c8ca; font-size: 12px; }.agent-fieldset { display: grid; gap: 8px; margin: 0; padding: 10px; border: 1px solid #34434a; border-radius: 5px; }.agent-fieldset legend { padding: 0 5px; color: #8fa1a5; font-size: 11px; }.agent-option { display: grid; grid-template-columns: minmax(0, 1fr) 135px; align-items: center; gap: 10px; color: #d7e0e2; font-size: 12px; }.agent-toggle { display: flex; align-items: center; gap: 7px; }.agent-toggle input { accent-color: #75b9a4; }.agent-lookback { min-width: 0; padding: 6px 8px; border: 1px solid #34434a; border-radius: 5px; background: #10171c; color: #d7e0e2; font: inherit; font-size: 11px; }.agent-lookback:disabled { opacity: .5; }.scan-field { display: grid; gap: 7px; color: #d7e0e2; font-size: 12px; }.scan-field input { width: 150px; padding: 8px; border: 1px solid #34434a; border-radius: 5px; background: #10171c; color: #e6eeed; font: inherit; }.field-help { margin: 0; color: #7f9198; font-size: 11px; line-height: 1.45; }.secondary-button { width: max-content; border: 1px solid #3c4d53; border-radius: 5px; padding: 7px 10px; color: #b9c8c8; background: transparent; cursor: pointer; font: inherit; font-size: 11px; }.secondary-button:disabled { opacity: .5; cursor: not-allowed; }
</style>
