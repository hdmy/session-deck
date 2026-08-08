<script setup lang="ts">
import { computed } from 'vue';
import type { SessionDetail } from '../../types';

const props = defineProps<{ detail: SessionDetail }>();
const branches = computed(() => props.detail.branches ?? []);
const selected = computed(() => branches.value.find((branch) => branch.id === props.detail.selected_branch_id));
const alternateCount = computed(() => branches.value.filter((branch) => !branch.is_active && branch.kind !== 'active').length);
const compacted = computed(() => props.detail.timeline.some((event) => event.compact_boundary) || Boolean(selected.value?.compacted));
const relations = computed(() => props.detail.relations ?? []);
const cwdHistory = computed(() => [...(props.detail.cwd_history ?? [])].sort((a, b) => a.first_sequence - b.first_sequence));
const cwdChanged = computed(() => cwdHistory.value.length > 1);
function shortUuid(value: string | null | undefined): string {
  if (!value) return '';
  return value.length > 12 ? `${value.slice(0, 8)}…${value.slice(-4)}` : value;
}
function date(value: number | null): string { return value ? new Date(value).toLocaleString() : '时间未知'; }
const text = computed(() => {
  const hints: string[] = [];
  if (selected.value?.kind === 'alternate' || selected.value?.is_active === false) {
    hints.push('当前为 alternate 分支：只读恢复视图（只读历史分支视图/恢复浏览），不会改写 transcript，也不支持任意历史点续聊。');
    if (selected.value.fork_point_uuid) hints.push(`分叉点 ${shortUuid(selected.value.fork_point_uuid)}。`);
    hints.push(`此分支范围：${selected.value.event_count} 个事件、${selected.value.turn_count} 个 turn。`);
  }
  else if (selected.value?.is_active || (
    props.detail.selected_branch_id !== undefined
    && props.detail.selected_branch_id !== null
    && props.detail.active_branch_id !== undefined
    && props.detail.active_branch_id !== null
    && props.detail.selected_branch_id === props.detail.active_branch_id
  )) hints.push('当前为主分支。');
  if (selected.value?.is_active && alternateCount.value > 0) hints.push(`主分支外还有 ${alternateCount.value} 个历史分支可浏览。`);
  if (compacted.value) hints.push('此历史包含 compact 边界，边界前的上下文可能已被压缩；这里只读显示。');
  for (const relation of relations.value) {
    const isParent = relation.parent_session_id === props.detail.summary.id;
    const other = shortUuid(isParent ? relation.child_session_id : relation.parent_session_id);
    const direction = isParent ? '子 session' : '父 session';
    const presence = isParent ? relation.child_present : relation.parent_present;
    hints.push(`${direction} ${other} · provider ${relation.provider_id} · relation ${relation.relation_type} · status ${relation.status}${presence ? '' : '（source 未找到）'}`);
  }
  return hints;
});
</script>

<template>
  <aside v-if="text.length || cwdChanged" class="history-hint" role="note" aria-label="History information">
    <span v-for="(item, index) in text" :key="index">{{ item }}</span>
    <details v-if="cwdChanged" class="cwd-history">
      <summary>历史 cwd 变化（{{ cwdHistory.length }}）</summary>
      <ol>
        <li v-for="item in cwdHistory" :key="`${item.cwd}-${item.first_sequence}`">
          <code>{{ item.cwd }}</code>
          <span v-if="item.resume">后续 cwd</span>
          <span>{{ date(item.first_timestamp) }}</span>
        </li>
      </ol>
    </details>
  </aside>
</template>

<style scoped>
.cwd-history { flex-basis: 100%; color: #b9a47b; }.cwd-history summary { width: max-content; cursor: pointer; outline: none; }.cwd-history summary:focus-visible { outline: 2px solid #75b9a4; outline-offset: 2px; }.cwd-history ol { display: grid; gap: 4px; margin: 6px 0 0; padding-left: 20px; }.cwd-history li { display: flex; flex-wrap: wrap; gap: 7px; }.cwd-history code { color: #d1dedd; overflow-wrap: anywhere; }.cwd-history li span { color: #8e8060; }.cwd-history li span::before { content: ''; }
</style>
