<script setup lang="ts">
import { computed, onScopeDispose, shallowRef } from 'vue';
import { highlightSanitizedHtml, renderSafeMarkdown } from './markdownSecurity';
import { copyText } from './copyText';

const props = withDefaults(defineProps<{ content: string; defaultMode?: 'rendered' | 'raw'; query?: string }>(), { defaultMode: 'rendered', query: '' });
const mode = shallowRef<'rendered' | 'raw'>(props.defaultMode);
const rendered = computed(() => highlightSanitizedHtml(renderSafeMarkdown(props.content), props.query));
const copyStatus = shallowRef<'copied' | 'error' | null>(null);
let copyStatusTimer: number | undefined;

async function copy() {
  copyStatus.value = (await copyText(props.content)) ? 'copied' : 'error';
  if (copyStatusTimer !== undefined) window.clearTimeout(copyStatusTimer);
  copyStatusTimer = window.setTimeout(() => { copyStatus.value = null; }, 1200);
}

onScopeDispose(() => { if (copyStatusTimer !== undefined) window.clearTimeout(copyStatusTimer); });
</script>

<template>
  <section class="markdown-content">
    <header class="markdown-toolbar">
      <div class="markdown-mode" role="group" aria-label="Response format">
        <button type="button" :class="{ active: mode === 'rendered' }" :aria-pressed="mode === 'rendered'" @click="mode = 'rendered'">Rendered</button>
        <button type="button" :class="{ active: mode === 'raw' }" :aria-pressed="mode === 'raw'" @click="mode = 'raw'">Raw</button>
      </div>
      <button type="button" class="copy-markdown" :aria-describedby="copyStatus === 'error' ? 'copy-markdown-error' : undefined" @click="copy">{{ copyStatus === 'copied' ? 'Copied' : copyStatus === 'error' ? 'Copy failed' : 'Copy' }}</button>
      <span v-if="copyStatus === 'error'" id="copy-markdown-error" class="copy-error" role="alert">Clipboard unavailable</span>
    </header>
    <div v-if="mode === 'rendered'" class="markdown-body" data-testid="safe-markdown" v-html="rendered"></div>
    <pre v-else class="markdown-raw">{{ content }}</pre>
  </section>
</template>

<style scoped>
.markdown-content { border-left: 1px solid #35454c; padding-left: 16px; }
.markdown-toolbar { display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px; }
.markdown-mode { display: inline-flex; gap: 2px; padding: 2px; background: #1d272d; border-radius: 5px; }
.markdown-mode button, .copy-markdown { border: 0; background: transparent; color: #829198; border-radius: 4px; padding: 4px 7px; font: inherit; font-size: 10px; cursor: pointer; }
.markdown-mode button.active, .markdown-mode button:hover, .copy-markdown:hover { color: #c6e2d8; background: #2a3d40; }
.copy-markdown { border: 1px solid #35454c; }
.markdown-body { color: #d5dde3; font-size: 13px; line-height: 1.7; overflow-wrap: anywhere; }
.markdown-body :deep(h1), .markdown-body :deep(h2), .markdown-body :deep(h3), .markdown-body :deep(h4) { color: #eef4f1; line-height: 1.3; margin: 1.15em 0 .45em; }
.markdown-body :deep(h1) { font-size: 1.4em; } .markdown-body :deep(h2) { font-size: 1.2em; } .markdown-body :deep(h3) { font-size: 1.05em; }
.markdown-body :deep(p), .markdown-body :deep(ul), .markdown-body :deep(ol), .markdown-body :deep(blockquote), .markdown-body :deep(table) { margin: .65em 0; }
.markdown-body :deep(ul), .markdown-body :deep(ol) { padding-left: 1.5em; }
.markdown-body :deep(blockquote) { border-left: 2px solid #75b9a4; color: #9eafb3; padding-left: 12px; }
.markdown-body :deep(pre) { overflow-x: auto; padding: 12px; background: #0d1216; border: 1px solid #28343b; border-radius: 5px; }
.markdown-body :deep(code) { color: #d4c28a; background: #202b31; border-radius: 3px; padding: 1px 4px; font-size: .92em; }
.markdown-body :deep(pre code) { padding: 0; background: transparent; color: #d5dde3; }
.markdown-body :deep(table) { width: 100%; border-collapse: collapse; font-size: 12px; }
.markdown-body :deep(th), .markdown-body :deep(td) { border: 1px solid #344149; padding: 6px 8px; text-align: left; }
.markdown-body :deep(th) { color: #c6e2d8; background: #1b282d; }
.markdown-raw { margin: 0; max-height: 520px; overflow: auto; white-space: pre-wrap; color: #aebbc0; font: inherit; font-size: 12px; line-height: 1.6; }
</style>
