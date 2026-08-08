<script setup lang="ts">
const props = defineProps<{
  intervalSeconds: number;
  disabled?: boolean;
  disabledReason?: string | null;
  saving?: boolean;
}>();
const emit = defineEmits<{ 'update:intervalSeconds': [value: number]; save: [] }>();
</script>

<template>
  <section class="scan-section" aria-labelledby="scan-schedule-title">
    <h3 id="scan-schedule-title">自动扫描</h3>
    <label class="scan-field">间隔（秒）
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
    <p id="scan-interval-help" class="field-help">0 表示停用；启用时只能使用后端允许的 60–3600 秒范围。</p>
    <p v-if="props.disabled && props.disabledReason" id="scan-disabled-reason" class="field-help" role="status">{{ props.disabledReason }}</p>
    <button class="secondary-button" type="button" :disabled="props.disabled || props.saving" @click="emit('save')">{{ props.saving ? '保存中…' : '保存扫描设置' }}</button>
  </section>
</template>

<style scoped>
.scan-section { display: grid; gap: 9px; }.scan-section h3 { margin: 0; color: #b8c8ca; font-size: 12px; }.scan-field { display: grid; gap: 7px; color: #d7e0e2; font-size: 12px; }.scan-field input { width: 150px; padding: 8px; border: 1px solid #34434a; border-radius: 5px; background: #10171c; color: #e6eeed; font: inherit; }.field-help { margin: 0; color: #7f9198; font-size: 11px; line-height: 1.45; }.secondary-button { width: max-content; border: 1px solid #3c4d53; border-radius: 5px; padding: 7px 10px; color: #b9c8c8; background: transparent; cursor: pointer; font: inherit; font-size: 11px; }.secondary-button:disabled { opacity: .5; cursor: not-allowed; }
</style>
