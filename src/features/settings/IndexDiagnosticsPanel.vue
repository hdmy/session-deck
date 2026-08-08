<script setup lang="ts">
import type { IndexDiagnostics } from '../../types';
const props = defineProps<{ diagnostics: IndexDiagnostics | null; loading: boolean; error: string | null }>();
function date(value: number | null | undefined) { return value ? new Date(value).toLocaleString() : '暂无'; }
</script>

<template>
  <section class="diagnostics" aria-labelledby="diagnostics-title">
    <h3 id="diagnostics-title">索引诊断</h3>
    <p v-if="props.loading" class="field-help" role="status">加载诊断中…</p>
    <p v-else-if="props.error" class="diagnostics-error" role="alert">{{ props.error }}</p>
    <p v-else-if="!props.diagnostics" class="field-help">暂无扫描诊断。</p>
    <template v-else>
      <dl class="diagnostics-list">
        <dt>Effective root</dt><dd>{{ props.diagnostics.effective_root }}</dd>
        <dt>Indexed sessions</dt><dd>{{ props.diagnostics.indexed_sessions }}</dd>
        <dt>Last attempt</dt><dd>{{ date(props.diagnostics.last_attempt_at) }}</dd>
        <dt>Last success</dt><dd>{{ date(props.diagnostics.last_success_at) }}</dd>
        <dt>Last outcome</dt><dd>{{ props.diagnostics.last_outcome ?? '暂无' }}</dd>
      </dl>
      <ul v-if="props.diagnostics.diagnostic_counts.length" class="diagnostic-codes">
        <li v-for="item in props.diagnostics.diagnostic_counts" :key="item.code"><code>{{ item.code }}</code><span>{{ item.count }}</span></li>
      </ul>
      <p v-else class="field-help">最近扫描没有聚合诊断。</p>
    </template>
  </section>
</template>

<style scoped>
.diagnostics { display: grid; gap: 9px; }.diagnostics h3 { margin: 0; color: #b8c8ca; font-size: 12px; }.field-help { margin: 0; color: #7f9198; font-size: 11px; line-height: 1.45; }.diagnostics-error { margin: 0; color: #dc9292; font-size: 11px; }.diagnostics-list { display: grid; grid-template-columns: 130px minmax(0,1fr); gap: 6px 10px; margin: 0; padding: 10px; border: 1px solid #2c3b42; border-radius: 5px; background: #11181d; font-size: 11px; }.diagnostics-list dt { color: #778a92; }.diagnostics-list dd { margin: 0; color: #d1dedd; overflow-wrap: anywhere; }.diagnostic-codes { display: grid; gap: 5px; margin: 0; padding: 0; list-style: none; }.diagnostic-codes li { display: flex; justify-content: space-between; gap: 10px; color: #c7d4d5; font-size: 11px; }.diagnostic-codes code { color: #d4c28a; }
</style>
