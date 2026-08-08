<script setup lang="ts">
import { computed } from 'vue';
import type { Project, SearchHit } from '../../types';
import ProjectGroup from './ProjectGroup.vue';
import SearchBox from './SearchBox.vue';
import HiddenSessionsPanel from './HiddenSessionsPanel.vue';
import { hitsBySession, projectsForQuery } from './navigation';

const props = defineProps<{
  projects: Project[];
  selectedId: string | null;
  query: string;
  hits: SearchHit[];
  scanning: boolean;
  searching: boolean;
  error: string | null;
  hiddenSessions: readonly import('../../types').SessionSummary[];
  hiddenLoading: boolean;
  hiddenError: string | null;
  mutationLoading: string | null;
  mutationError: string | null;
  activeContinuationSessionId?: string | null;
  activeContinuationTargetSessionId?: string | null;
  continuationActive?: boolean;
  aliasBusy?: string | null;
}>();

const emit = defineEmits<{
  search: [value: string];
  select: [id: string, eventId?: number];
  refresh: [];
  settings: [];
  pin: [id: string, value: boolean];
  hide: [id: string];
  rename: [id: string, title: string | null];
  restore: [id: string];
  alias: [id: string, alias: string | null];
}>();

const visibleProjects = computed(() => projectsForQuery(props.projects, props.hits, props.query));
const matches = computed(() => hitsBySession(props.hits));
</script>

<template>
  <aside class="nav-pane" aria-label="Session navigation">
    <div class="brand"><span class="brand-mark">✦</span><span>Context Vault</span></div>
    <SearchBox :model-value="query" @update:model-value="emit('search', $event)" />

    <div v-if="error" class="nav-notice error" role="alert">{{ error }}</div>

    <div class="projects">
      <div class="section-label">
        <span>Projects</span>
        <span v-if="searching" role="status">Searching…</span>
        <span v-else-if="query.trim()">{{ hits.length }} matches</span>
      </div>

      <ProjectGroup
        v-for="project in visibleProjects"
        :key="project.id"
        :project="project"
        :selected-id="selectedId"
        :matches="matches"
        :force-open="Boolean(query.trim())"
        :busy-id="mutationLoading"
        :active-continuation-session-id="activeContinuationSessionId ?? null"
        :active-continuation-target-session-id="activeContinuationTargetSessionId ?? null"
        :alias-busy="aliasBusy"
        @select="emit('select', $event[0], $event[1])"
        @pin="emit('pin', $event[0], $event[1])"
        @hide="emit('hide', $event)"
        @rename="emit('rename', $event[0], $event[1])"
        @alias="emit('alias', $event[0], $event[1])"
      />

      <div v-if="!visibleProjects.length && !scanning && !searching" class="empty-mini">
        {{ query.trim() ? 'No matches' : 'No Claude Code sessions found' }}
      </div>

      <div v-if="mutationError" class="nav-notice error" role="alert">{{ mutationError }}</div>
      <HiddenSessionsPanel
        :sessions="hiddenSessions"
        :loading="hiddenLoading"
        :error="hiddenError"
        :busy-id="mutationLoading"
        :active-continuation-session-id="activeContinuationSessionId ?? null"
        :active-continuation-target-session-id="activeContinuationTargetSessionId ?? null"
        @restore="emit('restore', $event)"
      />
    </div>

    <div class="nav-footer">
      <button class="refresh-button" :disabled="scanning || continuationActive" :aria-describedby="continuationActive ? 'refresh-disabled-reason' : undefined" type="button" @click="emit('refresh')">
        {{ scanning ? 'Scanning…' : continuationActive ? 'Refresh unavailable during continuation' : 'Refresh index' }}
      </button>
      <span v-if="continuationActive" id="refresh-disabled-reason" class="refresh-disabled-reason" role="status">续聊进行中，结束后可刷新索引</span>
      <button class="settings-button" type="button" aria-label="Open Claude settings" @click="emit('settings')">⚙</button>
      <span>Local index · source files stay untouched</span>
    </div>
  </aside>
</template>
