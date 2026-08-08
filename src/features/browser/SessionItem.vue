<script setup lang="ts">
import { computed, shallowRef, useTemplateRef } from 'vue';
import type { SearchHit, SessionSummary } from '../../types';

const props = defineProps<{
  session: SessionSummary;
  active: boolean;
  match?: SearchHit;
  busy?: boolean;
  continuationActive?: boolean;
  continuationTargetActive?: boolean;
}>();

const emit = defineEmits<{
  select: [];
  pin: [value: boolean];
  hide: [];
  rename: [title: string | null];
}>();

const editing = shallowRef(false);
const draft = shallowRef('');
const renameInput = useTemplateRef<HTMLInputElement>('renameInput');
const tooltipId = computed(() => `session-meta-${props.session.id.replace(/[^a-zA-Z0-9_-]/g, '-')}`);
const continuationReasonId = computed(() => `${tooltipId.value}-continuation`);
const continuationLocked = computed(() => Boolean(props.continuationActive || props.continuationTargetActive));
const mutationLocked = computed(() => Boolean(props.busy) || continuationLocked.value);

function date(value: number | null): string {
  return value
    ? new Date(value).toLocaleString([], {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
      })
    : 'Unknown time';
}

function beginRename() {
  if (mutationLocked.value) return;
  draft.value = props.session.title;
  editing.value = true;
  requestAnimationFrame(() => {
    renameInput.value?.focus();
    renameInput.value?.select();
  });
}

function cancelRename() {
  editing.value = false;
}

function saveRename() {
  if (mutationLocked.value) return;
  const title = draft.value.trim();
  emit('rename', title && title !== props.session.source_title ? title : null);
  editing.value = false;
}

function onRenameKeydown(event: KeyboardEvent) {
  if (event.isComposing || event.keyCode === 229) return;
  if (event.key === 'Enter') {
    event.preventDefault();
    saveRename();
  }
  if (event.key === 'Escape') {
    event.preventDefault();
    cancelRename();
  }
}
</script>

<template>
  <div
    class="session-item-row tooltip-anchor"
    :class="{ active }"
    :aria-busy="busy"
  >
    <button
      class="session-item"
      type="button"
      :aria-current="active ? 'page' : undefined"
      :aria-label="`${session.title}, ${date(session.ended_at)}, ${session.branch || 'no branch'}`"
      :aria-describedby="tooltipId"
      @click="emit('select')"
    >
      <span class="session-dot" aria-hidden="true"></span>
      <span class="session-copy">
        <strong>{{ session.title }}</strong>
        <small v-if="match">{{ match.snippet }}</small>
      </span>
      <span v-if="continuationActive" class="continuation-mark" role="status">续聊中</span>
      <span v-if="session.pinned" class="pin-mark" title="Pinned" aria-label="Pinned">★</span>
      <span v-if="session.partial" class="partial-mark" title="Partial parse">~</span>
    </button>

    <form v-if="editing" class="session-edit-form" @submit.prevent="saveRename">
      <input
        ref="renameInput"
        v-model="draft"
        class="session-rename-input"
        aria-label="Session title"
        @keydown="onRenameKeydown"
      />
      <button type="submit" class="session-action" :disabled="mutationLocked" :aria-describedby="mutationLocked ? continuationReasonId : undefined" aria-label="Save title">✓</button>
      <button type="button" class="session-action" :disabled="mutationLocked" :aria-describedby="mutationLocked ? continuationReasonId : undefined" aria-label="Cancel rename" @click="cancelRename">×</button>
    </form>

    <div v-else class="session-actions" role="group" :aria-label="`Actions for ${session.title}`">
      <button
        type="button"
        class="session-action"
        :disabled="mutationLocked"
        :aria-describedby="mutationLocked ? continuationReasonId : undefined"
        :aria-label="session.pinned ? 'Unpin session' : 'Pin session'"
        :title="session.pinned ? '取消置顶' : '置顶'"
        @click="emit('pin', !session.pinned)"
      >{{ session.pinned ? '★' : '☆' }}</button>
      <button type="button" class="session-action" :disabled="mutationLocked" :aria-describedby="mutationLocked ? continuationReasonId : undefined" aria-label="Rename session" title="重命名" @click="beginRename">✎</button>
      <button
        v-if="session.title !== session.source_title"
        type="button"
        class="session-action"
        :disabled="mutationLocked"
        :aria-describedby="mutationLocked ? continuationReasonId : undefined"
        aria-label="Reset session title"
        title="恢复原标题"
        @click="emit('rename', null)"
      >↺</button>
      <button type="button" class="session-action hide-action" :disabled="mutationLocked" :aria-describedby="mutationLocked ? continuationReasonId : undefined" aria-label="隐藏会话" @click="emit('hide')">隐藏</button>
    </div>

    <span v-if="mutationLocked" :id="continuationReasonId" class="session-disabled-reason" role="status">
      {{ props.continuationTargetActive ? '分叉子会话续聊中，结束后可隐藏、重命名或置顶' : continuationLocked ? '续聊进行中，结束后可隐藏、重命名或置顶' : '会话更新进行中，请稍候' }}
    </span>

    <span :id="tooltipId" class="metadata-tooltip session-tooltip" role="tooltip">
      <strong>{{ date(session.ended_at) }}</strong>
      <span>Branch · {{ session.branch || 'Unavailable' }}</span>
      <span>Last input · {{ session.last_prompt || 'Unavailable' }}</span>
    </span>
  </div>
</template>
