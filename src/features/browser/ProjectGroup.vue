<script setup lang="ts">
import { computed, shallowRef } from 'vue';
import type { Project, SearchHit } from '../../types';
import SessionItem from './SessionItem.vue';

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
  alias: [selection: [id: string, alias: string | null]];
}>();

const open = shallowRef(true);
const pathExpanded = shallowRef(false);
const aliasEditing = shallowRef(false);
const aliasDraft = shallowRef('');
const expanded = computed(() => props.forceOpen || open.value);
const tooltipId = computed(
  () => `project-meta-${props.project.id.replace(/[^a-zA-Z0-9_-]/g, '-')}`,
);

function date(value: number | null): string {
  return value
    ? new Date(value).toLocaleString([], {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
      })
    : 'Unknown';
}
function beginAlias() { aliasDraft.value = props.project.alias ?? ''; aliasEditing.value = true; }
function submitAlias() { const next = aliasDraft.value.trim() || null; aliasEditing.value = false; emit('alias', [props.project.workspace_id || props.project.id, next]); }
</script>

<template>
  <section class="project-group">
    <button
      class="project-head tooltip-anchor"
      type="button"
      :aria-expanded="expanded"
      :aria-label="`${project.name}, ${project.sessions.length} sessions`"
      :aria-describedby="tooltipId"
      @click="open = !open"
    >
      <span class="chevron" aria-hidden="true">{{ expanded ? '▾' : '▸' }}</span>
      <span class="project-name">{{ project.alias || project.name }}</span>
      <span class="project-count">{{ project.sessions.length }}</span>
      <span :id="tooltipId" class="metadata-tooltip" role="tooltip">
        <strong>{{ project.alias || project.name }}</strong>
        <span>{{ project.path || 'Path unavailable' }}</span>
        <span>Recent activity · {{ date(project.latest_activity) }}</span>
      </span>
    </button>

    <div class="project-actions">
      <button v-if="!aliasEditing" class="alias-action" type="button" @click="beginAlias">Edit local alias</button>
      <span v-else class="alias-editor">
        <input v-model="aliasDraft" aria-label="Project alias" @keydown.enter.prevent="submitAlias" @keydown.esc.prevent="aliasEditing = false" />
        <button type="button" :disabled="aliasBusy === (project.workspace_id || project.id)" @click="submitAlias">{{ aliasBusy === (project.workspace_id || project.id) ? '…' : 'OK' }}</button>
      </span>
    </div>

    <button class="path-toggle" type="button" :aria-expanded="pathExpanded" @click="pathExpanded = !pathExpanded">
      {{ pathExpanded ? 'Hide paths' : 'Show project paths' }}
    </button>
    <div v-if="pathExpanded" class="project-paths">
      <div><strong>Stable project path</strong><code>{{ project.path || 'Unavailable' }}</code></div>
      <div><strong>Historical cwd paths</strong><code v-for="path in (project.cwd_paths ?? [])" :key="`cwd-${path}`">{{ path }}</code><span v-if="!(project.cwd_paths ?? []).length">None recorded</span></div>
      <div><strong>Local worktree paths</strong><code v-for="path in (project.worktree_paths ?? [])" :key="`worktree-${path}`">{{ path }}</code><span v-if="!(project.worktree_paths ?? []).length">None detected</span></div>
      <p>Sessions are grouped by local project identity; historical cwd is not inferred as a worktree.</p>
    </div>

    <div v-if="expanded" class="session-list">
      <SessionItem
        v-for="session in project.sessions"
        :key="session.id"
        :session="session"
        :active="selectedId === session.id"
        :match="matches.get(session.id)"
        :busy="Boolean(busyId)"
        :continuation-active="activeContinuationSessionId === session.id"
        :continuation-target-active="activeContinuationTargetSessionId === session.id"
        @select="emit('select', [session.id, matches.get(session.id)?.event_id])"
        @pin="emit('pin', [session.id, $event])"
        @hide="emit('hide', session.id)"
        @rename="emit('rename', [session.id, $event])"
      />
    </div>
  </section>
</template>

<style scoped>
.project-actions { display: flex; justify-content: flex-end; margin: 3px 10px 0 26px; }.alias-action { border: 0; color: #74868d; background: transparent; cursor: pointer; font: inherit; font-size: 10px; }.alias-action:hover, .alias-action:focus-visible { color: #c9e8dd; }.alias-editor { display: inline-flex; gap: 3px; }.alias-editor input { width: 120px; padding: 3px 5px; border: 1px solid #49625f; border-radius: 3px; background: #10171c; color: #e6eeed; font: inherit; font-size: 11px; }.alias-editor button { border: 1px solid #3c4d53; border-radius: 3px; background: transparent; color: #b9c8c8; font-size: 10px; }.path-toggle { margin: 4px 10px 0 26px; border: 0; color: #7f9198; background: transparent; cursor: pointer; font: inherit; font-size: 10px; text-align: left; }.project-paths { display: grid; gap: 7px; margin: 4px 10px 9px 26px; padding: 9px; border: 1px solid #2c3b42; border-radius: 5px; background: #11181d; color: #9baeb1; font-size: 10px; }.project-paths div { display: grid; gap: 3px; }.project-paths strong { color: #778a92; font-weight: 500; }.project-paths code { color: #d1dedd; overflow-wrap: anywhere; }.project-paths p { margin: 0; color: #7f9198; line-height: 1.4; }
</style>
