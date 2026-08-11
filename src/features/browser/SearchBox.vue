<script setup lang="ts">
import { useI18n } from '../../i18n';

defineProps<{ modelValue: string }>();
const emit = defineEmits<{ 'update:modelValue': [value: string] }>();
const { t } = useI18n();
</script>

<template>
  <label class="search-box">
    <span aria-hidden="true">⌕</span>
    <input
      :value="modelValue"
      :aria-label="t('searchAcrossProjectsAndSessions')"
      :placeholder="t('searchAllSessions')"
      @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
    />
    <kbd>⌘K</kbd>
  </label>
</template>
