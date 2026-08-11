<script setup lang="ts">
import { computed, onScopeDispose, shallowRef, useTemplateRef } from 'vue';
import type { Project, ProviderId, SearchHit } from '../../types';
import SessionItem from './SessionItem.vue';
import { formatDateTime, useI18n } from '../../i18n';

const props = defineProps<{
  project: Project;
  selectedId: string | null;
  matches: ReadonlyMap<string, SearchHit>;
  forceOpen: boolean;
  busyId?: string | null;
  activeContinuationSessionId?: string | null;
  activeContinuationTargetSessionId?: string | null;
  aliasBusy?: string | null;
}>();

const emit = defineEmits<{
  select: [selection: [id: string, eventId?: number]];
  pin: [selection: [id: string, value: boolean]];
  hide: [id: string];
  rename: [selection: [id: string, title: string | null]];
  alias: [selection: [providerId: ProviderId, workspaceId: string, alias: string | null]];
}>();

const open = shallowRef(true);
const pathExpanded = shallowRef(false);
const aliasEditing = shallowRef(false);
const aliasDraft = shallowRef('');
const aliasInput = useTemplateRef<HTMLInputElement>('aliasInput');
const headRowRef = useTemplateRef<HTMLElement>('headRowRef');
const showTooltip = shallowRef(false);
const tooltipStyle = shallowRef<Record<string, string>>({});
const { t } = useI18n();

const expanded = computed(() => props.forceOpen || open.value);
const aliasEditable = computed(() => !props.project.agents || props.project.agents.length <= 1);
const tooltipId = computed(
  () => `project-meta-${props.project.id.replace(/[^a-zA-Z0-9_-]/g, '-')}`,
);

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
  if (!headRowRef.value) return;
  const rect = headRowRef.value.getBoundingClientRect();
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

const showDisabledNotice = shallowRef(false);
let disabledNoticeTimer: number | undefined;

function beginAlias() {
  if (!aliasEditable.value) {
    showDisabledNotice.value = true;
    if (disabledNoticeTimer !== undefined) window.clearTimeout(disabledNoticeTimer);
    disabledNoticeTimer = window.setTimeout(() => {
      showDisabledNotice.value = false;
    }, 2000);
    return;
  }
  aliasDraft.value = props.project.alias || props.project.name;
  aliasEditing.value = true;
  requestAnimationFrame(() => {
    aliasInput.value?.focus();
    aliasInput.value?.select();
  });
}

function submitAlias() {
  if (!aliasEditable.value) return;
  const next = aliasDraft.value.trim() || null;
  aliasEditing.value = false;
  const providerId = props.project.agents?.[0]?.provider_id
    ?? props.project.provider_ids?.[0]
    ?? props.project.sessions[0]?.provider_id;
  if (providerId) emit('alias', [providerId, props.project.workspace_id || props.project.id, next]);
}

function cancelAlias() {
  aliasEditing.value = false;
}

onScopeDispose(() => {
  if (disabledNoticeTimer !== undefined) window.clearTimeout(disabledNoticeTimer);
});
</script>

<template>
  <section class="project-group">
    <div
      ref="headRowRef"
      class="project-head-row tooltip-anchor"
      @mouseenter="onMouseEnter"
      @mouseleave="onMouseLeave"
      @focusin="onMouseEnter"
      @focusout="onMouseLeave"
    >
      <button
        class="project-head"
        type="button"
        :aria-expanded="expanded"
        :aria-label="t('projectSummary', { name: project.name, count: project.sessions.length })"
        :aria-describedby="tooltipId"
        @click="open = !open"
      >
        <span class="chevron" aria-hidden="true">{{ expanded ? '▾' : '▸' }}</span>
        <span v-if="!aliasEditing" class="project-name">{{ project.alias || project.name }}</span>
        <form v-else class="alias-edit-form" @submit.prevent="submitAlias" @click.stop>
          <input
            ref="aliasInput"
            v-model="aliasDraft"
            class="project-alias-input"
            :aria-label="t('projectAlias')"
            :placeholder="t('projectAlias')"
            @keydown.enter.prevent="submitAlias"
            @keydown.esc.prevent="cancelAlias"
          />
          <button
            type="submit"
            class="project-action"
            :disabled="aliasBusy === (project.workspace_id || project.id)"
            :aria-label="t('saveAlias')"
            :title="t('saveAlias')"
          >
            {{ aliasBusy === (project.workspace_id || project.id) ? '…' : '✓' }}
          </button>
          <button
            type="button"
            class="project-action"
            :disabled="aliasBusy === (project.workspace_id || project.id)"
            :aria-label="t('cancelEdit')"
            :title="t('cancelEdit')"
            @click="cancelAlias"
          >
            ×
          </button>
        </form>
        <span v-if="!aliasEditing" class="project-head-actions" role="group" :aria-label="t('actionsFor', { title: project.name })">
          <button
            type="button"
            class="project-action alias-action"
            :class="{ disabled: !aliasEditable }"
            :title="aliasEditable ? t('editProjectAlias') : t('filterAgentBeforeAlias')"
            :aria-label="aliasEditable ? t('editProjectAlias') : t('filterAgentBeforeAlias')"
            @click.stop="beginAlias"
          >✎</button>
          <span v-if="showDisabledNotice" class="alias-disabled-popover" role="status">
            {{ t('filterAgentBeforeAlias') }}
          </span>
        </span>
        <span class="project-count">{{ project.sessions.length }}</span>
      </button>
    </div>

    <Teleport to="body">
      <div
        v-if="showTooltip"
        :id="tooltipId"
        class="metadata-tooltip-right project-tooltip-right"
        :style="tooltipStyle"
        role="tooltip"
      >
        <strong>{{ project.alias || project.name }}</strong>
        <div class="tooltip-section">
          <span class="tooltip-label">{{ t('stableProjectPath') }}</span>
          <code>{{ project.path || t('unavailable') }}</code>
        </div>
        <div class="tooltip-section">
          <span class="tooltip-label">{{ t('historicalCwdPaths') }}</span>
          <code v-for="path in (project.cwd_paths ?? [])" :key="`cwd-${path}`">{{ path }}</code>
          <span v-if="!(project.cwd_paths ?? []).length" class="tooltip-muted">{{ t('noneRecorded') }}</span>
        </div>
        <div class="tooltip-section">
          <span class="tooltip-label">{{ t('localWorktreePaths') }}</span>
          <code v-for="path in (project.worktree_paths ?? [])" :key="`worktree-${path}`">{{ path }}</code>
          <span v-if="!(project.worktree_paths ?? []).length" class="tooltip-muted">{{ t('noneDetected') }}</span>
        </div>
        <div class="tooltip-meta">{{ t('recentActivity') }} · {{ date(project.latest_activity) }}</div>
      </div>
    </Teleport>

    <div v-if="expanded" class="session-list">
      <SessionItem
        v-for="session in project.sessions"
        :key="`${session.provider_id}:${session.id}`"
        :session="session"
        :active="selectedId === session.id"
        :match="matches.get(session.id) ?? matches.get(`${session.provider_id}:${session.id}`)"
        :busy="Boolean(busyId)"
        :continuation-active="activeContinuationSessionId === session.id"
        :continuation-target-active="activeContinuationTargetSessionId === session.id"
        @select="emit('select', [session.id, (matches.get(session.id) ?? matches.get(`${session.provider_id}:${session.id}`))?.event_id])"
        @pin="emit('pin', [session.id, $event])"
        @hide="emit('hide', session.id)"
        @rename="emit('rename', [session.id, $event])"
      />
    </div>
  </section>
</template>

<style scoped>
.project-paths { display: grid; gap: 7px; margin: 4px 10px 9px 26px; padding: 9px; border: 1px solid #2c3b42; border-radius: 5px; background: #11181d; color: #9baeb1; font-size: 10px; }.project-paths div { display: grid; gap: 3px; }.project-paths strong { color: #778a92; font-weight: 500; }.project-paths code { color: #d1dedd; overflow-wrap: anywhere; }
.project-head-actions { position: relative; }
.project-action.disabled { opacity: 0.5; cursor: not-allowed; }
.project-action.disabled:hover { background: transparent; color: #8a9a9e; }
.alias-disabled-popover {
  position: absolute;
  top: -26px;
  right: 0;
  z-index: 99;
  padding: 3px 8px;
  border-radius: 4px;
  background: #232d35;
  border: 1px solid #3c4c57;
  color: #e5c378;
  font-size: 10px;
  white-space: nowrap;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
  pointer-events: none;
  animation: popoverFadeIn 120ms ease-out;
}
@keyframes popoverFadeIn {
  from { opacity: 0; transform: translateY(3px); }
  to { opacity: 1; transform: translateY(0); }
}
</style>
