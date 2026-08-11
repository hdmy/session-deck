<script setup lang="ts">
import { computed } from 'vue';
import type { ProviderId } from '../../types';

import claudeSvg from '@lobehub/icons-static-svg/icons/claude-color.svg?raw';
import codexSvg from '@lobehub/icons-static-svg/icons/codex-color.svg?raw';
import geminiSvg from '@lobehub/icons-static-svg/icons/gemini-color.svg?raw';
import deepseekSvg from '@lobehub/icons-static-svg/icons/deepseek-color.svg?raw';
import opencodeSvg from '@lobehub/icons-static-svg/icons/opencode.svg?raw';
import qwenSvg from '@lobehub/icons-static-svg/icons/qwen-color.svg?raw';
import mistralSvg from '@lobehub/icons-static-svg/icons/mistral-color.svg?raw';
import antigravitySvg from '@lobehub/icons-static-svg/icons/antigravity-color.svg?raw';

const props = withDefaults(defineProps<{
  providerId: ProviderId | string;
  size?: number | string;
}>(), {
  size: 14,
});

const iconSvg = computed<string | null>(() => {
  const pid = props.providerId.toLowerCase();
  if (pid.includes('claude')) return claudeSvg;
  if (pid.includes('codex') || pid.includes('openai')) return codexSvg;
  if (pid.includes('gemini')) return geminiSvg;
  if (pid.includes('deepseek')) return deepseekSvg;
  if (pid.includes('opencode')) return opencodeSvg;
  if (pid.includes('qwen')) return qwenSvg;
  if (pid.includes('mistral')) return mistralSvg;
  if (pid.includes('antigravity')) return antigravitySvg;
  return null;
});

const iconSizeStyle = computed(() => {
  const s = typeof props.size === 'number' ? `${props.size}px` : props.size;
  return {
    width: s,
    height: s,
  };
});
</script>

<template>
  <span
    class="agent-icon"
    :style="iconSizeStyle"
    aria-hidden="true"
  >
    <span v-if="iconSvg" class="agent-svg" v-html="iconSvg"></span>
    <span v-else class="agent-fallback">{{ providerId.slice(0, 2).toUpperCase() }}</span>
  </span>
</template>

<style scoped>
.agent-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
  border-radius: 3px;
  overflow: hidden;
  vertical-align: middle;
}
.agent-svg {
  display: flex;
  width: 100%;
  height: 100%;
  align-items: center;
  justify-content: center;
}
.agent-svg :deep(svg) {
  width: 100%;
  height: 100%;
  display: block;
}
.agent-fallback {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  background: #2b3842;
  color: #a4b8c4;
  font-size: 8px;
  font-weight: 700;
  border-radius: 2px;
  text-transform: uppercase;
}
</style>
