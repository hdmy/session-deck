<script setup lang="ts">
defineProps<{ modelValue: string }>();
const emit = defineEmits<{ 'update:modelValue': [value: string] }>();
</script>

<template>
  <label class="search-box">
    <span aria-hidden="true">⌕</span>
    <input
      :value="modelValue"
      aria-label="Search across projects and sessions"
      placeholder="Search all sessions…"
      @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
    />
    <kbd>⌘K</kbd>
  </label>
</template>
