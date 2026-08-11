<script setup lang="ts">
import { computed, shallowRef, useTemplateRef } from 'vue';
import type { SearchHit, SessionSummary } from '../../types';
import AgentIcon from './AgentIcon.vue';
import { formatDateTime, useI18n } from '../../i18n';

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
const rowRef = useTemplateRef<HTMLElement>('rowRef');
const showTooltip = shallowRef(false);
const tooltipStyle = shallowRef<Record<string, string>>({});
const { t } = useI18n();

const tooltipId = computed(() => `session-meta-${props.session.id.replace(/[^a-zA-Z0-9_-]/g, '-')}`);
const continuationReasonId = computed(() => `${tooltipId.value}-continuation`);
const continuationLocked = computed(() => Boolean(props.continuationActive || props.continuationTargetActive));
const mutationLocked = computed(() => Boolean(props.busy) || continuationLocked.value);

function date(value: number | null): string {
  return value
    ? formatDateTime(value, {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
      })
    : t('unknownTime');
}

function updateTooltipPos() {
  if (!rowRef.value) return;
  const rect = rowRef.value.getBoundingClientRect();
  tooltipStyle.value = {
    position: 'fixed',
    top: `${rect.top + rect.height / 2}px`,
    left: `${rect.right + 8}px`,
    transform: 'translateY(-50%)',
    zIndex: '9999',
  };
}

function onMouseEnter() {
  updateTooltipPos();
  showTooltip.value = true;
}

function onMouseLeave() {
  showTooltip.value = false;
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
    ref="rowRef"
    class="session-item-row tooltip-anchor"
    :class="{ active, editing }"
    :aria-busy="busy"
    @mouseenter="onMouseEnter"
    @mouseleave="onMouseLeave"
    @focusin="onMouseEnter"
    @focusout="onMouseLeave"
  >
    <div v-if="editing" class="session-item-editing">
      <AgentIcon :provider-id="session.provider_id" :size="14" />
      <span class="session-dot" aria-hidden="true"></span>
      <form class="session-edit-form" @submit.prevent="saveRename" @click.stop>
        <input
          ref="renameInput"
          v-model="draft"
          class="session-rename-input"
          :aria-label="t('sessionTitle')"
          @keydown="onRenameKeydown"
        />
        <button
          type="submit"
          class="session-action"
          :disabled="mutationLocked"
          :aria-describedby="mutationLocked ? continuationReasonId : undefined"
          :aria-label="t('saveTitle')"
          :title="t('save')"
        >✓</button>
        <button
          type="button"
          class="session-action"
          :disabled="mutationLocked"
          :aria-describedby="mutationLocked ? continuationReasonId : undefined"
          :aria-label="t('cancelRename')"
          :title="t('cancel')"
          @click="cancelRename"
        >×</button>
      </form>
    </div>

    <button
      v-else
      class="session-item"
      type="button"
      :aria-current="active ? 'page' : undefined"
      :aria-label="`${session.title}, ${date(session.ended_at)}, ${session.branch || t('noBranch')}`"
      :aria-describedby="tooltipId"
      @click="emit('select')"
    >
      <AgentIcon :provider-id="session.provider_id" :size="14" />
      <span class="session-dot" aria-hidden="true"></span>
      <span class="session-copy">
        <strong>{{ session.title }}</strong>
        <small v-if="match">{{ match.snippet }}</small>
      </span>
      <span v-if="continuationActive" class="continuation-mark" role="status">{{ t('continuing') }}</span>
      <span v-if="session.pinned" class="pin-mark" :title="t('pinned')" :aria-label="t('pinned')">★</span>
      <span v-if="session.partial" class="partial-mark" :title="t('partialParse')">~</span>
    </button>

    <div v-if="!editing" class="session-actions" role="group" :aria-label="t('actionsFor', { title: session.title })">
      <button
        type="button"
        class="session-action"
        :disabled="mutationLocked"
        :aria-describedby="mutationLocked ? continuationReasonId : undefined"
        :aria-label="session.pinned ? t('unpinSession') : t('pinSession')"
        :title="session.pinned ? t('unpinSession') : t('pinSession')"
        @click="emit('pin', !session.pinned)"
      >{{ session.pinned ? '★' : '☆' }}</button>
      <button type="button" class="session-action" :disabled="mutationLocked" :aria-describedby="mutationLocked ? continuationReasonId : undefined" :aria-label="t('renameSession')" :title="t('renameSession')" @click="beginRename">✎</button>
      <button
        v-if="session.title !== session.source_title"
        type="button"
        class="session-action"
        :disabled="mutationLocked"
        :aria-describedby="mutationLocked ? continuationReasonId : undefined"
        :aria-label="t('resetSessionTitle')"
        :title="t('resetSessionTitle')"
        @click="emit('rename', null)"
      >↺</button>
      <button
        type="button"
        class="session-action hide-action"
        :disabled="mutationLocked"
        :aria-describedby="mutationLocked ? continuationReasonId : undefined"
        :aria-label="t('hideSession')"
        :title="t('hideSession')"
        @click="emit('hide')"
      >
        <svg class="action-icon" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <rect x="2" y="3" width="20" height="5" rx="1"/>
          <path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/>
          <path d="M10 12h4"/>
        </svg>
      </button>
    </div>

    <span v-if="mutationLocked" :id="continuationReasonId" class="session-disabled-reason" role="status">
      {{ props.continuationTargetActive ? t('forkSessionLocked') : continuationLocked ? t('continuationSessionLocked') : t('sessionUpdateInProgress') }}
    </span>

    <Teleport to="body">
      <div
        v-if="showTooltip && !editing"
        :id="tooltipId"
        class="metadata-tooltip-right session-tooltip-right"
        :style="tooltipStyle"
        role="tooltip"
      >
        <strong>{{ date(session.ended_at) }}</strong>
        <span>{{ t('branch') }} · {{ session.branch || t('unavailable') }}</span>
        <span>{{ t('lastInput') }} · {{ session.last_prompt || t('unavailable') }}</span>
      </div>
    </Teleport>
  </div>
</template>
