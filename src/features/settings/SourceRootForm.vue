<script setup lang="ts">
const props = defineProps<{
  sourceRoot: string;
  effectiveRoot: string;
  replaceConfirmed: boolean;
  disabled?: boolean;
  disabledReason?: string | null;
  saving?: boolean;
}>();
const emit = defineEmits<{
  'update:sourceRoot': [value: string];
  'update:replaceConfirmed': [value: boolean];
  activate: [];
}>();
</script>

<template>
  <section class="root-section" aria-labelledby="source-root-title">
    <h3 id="source-root-title">Claude source root</h3>
    <label class="root-field">Source root（留空使用默认）
      <input :value="props.sourceRoot" :disabled="props.disabled" autocomplete="off" @input="emit('update:sourceRoot', ($event.target as HTMLInputElement).value)" />
    </label>
    <p class="field-help">当前生效：<code>{{ props.effectiveRoot || '未配置' }}</code></p>
    <label class="confirm-row"><input type="checkbox" :checked="props.replaceConfirmed" :disabled="props.disabled" @change="emit('update:replaceConfirmed', ($event.target as HTMLInputElement).checked)" /> 我确认切换并替换当前本地索引</label>
    <p class="field-help">成功完整扫描才会原子替换派生索引；失败或 partial 不会按删除。</p>
    <p v-if="props.disabled && props.disabledReason" class="field-help" role="status">{{ props.disabledReason }}</p>
    <button class="secondary-button" type="button" :disabled="props.disabled || props.saving" @click="emit('activate')">{{ props.saving ? '扫描中…' : '激活并扫描此 root' }}</button>
  </section>
</template>

<style scoped>
.root-section { display: grid; gap: 9px; }.root-section h3 { margin: 0; color: #b8c8ca; font-size: 12px; }.root-field { display: grid; gap: 7px; color: #d7e0e2; font-size: 12px; }.root-field input { width: 100%; box-sizing: border-box; padding: 8px; border: 1px solid #34434a; border-radius: 5px; background: #10171c; color: #e6eeed; font: inherit; }.confirm-row { display: flex; gap: 7px; align-items: flex-start; color: #c7d4d5; font-size: 11px; }.confirm-row input { accent-color: #75b9a4; }.field-help { margin: 0; color: #7f9198; font-size: 11px; line-height: 1.45; }.field-help code { color: #d4c28a; overflow-wrap: anywhere; }.secondary-button { width: max-content; border: 1px solid #3c4d53; border-radius: 5px; padding: 7px 10px; color: #b9c8c8; background: transparent; cursor: pointer; font: inherit; font-size: 11px; }.secondary-button:disabled { opacity: .5; cursor: not-allowed; }
</style>
