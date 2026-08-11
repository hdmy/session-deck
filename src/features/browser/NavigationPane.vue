<script setup lang="ts">
import { computed } from 'vue';
import type { Project, ProviderDescriptor, ProviderId, SearchHit } from '../../types';
import ProjectGroup from './ProjectGroup.vue';
import SearchBox from './SearchBox.vue';
import ProviderFilter from './ProviderFilter.vue';
import HiddenSessionsPanel from './HiddenSessionsPanel.vue';
import { hitsBySession, projectsForQuery, sessionsForProvider } from './navigation';
import { useI18n } from '../../i18n';

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
  providers: readonly ProviderDescriptor[];
  providerId: ProviderId | null;
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
  alias: [providerId: ProviderId, workspaceId: string, alias: string | null];
  provider: [providerId: ProviderId | null];
}>();

const visibleProjects = computed(() => projectsForQuery(props.projects, props.hits, props.query, props.providerId));
const matches = computed(() => hitsBySession(props.hits));
const visibleHiddenSessions = computed(() => sessionsForProvider(props.hiddenSessions, props.providerId));
const { t } = useI18n();
</script>

<template>
  <aside class="nav-pane" :aria-label="t('sessionNavigation')">
    <div class="brand"><span class="brand-mark">✦</span><span>Context Vault</span></div>
    <SearchBox :model-value="query" @update:model-value="emit('search', $event)" />
    <ProviderFilter
      :providers="providers"
      :model-value="providerId"
      @update:model-value="emit('provider', $event)"
    />

    <div v-if="error" class="nav-notice error" role="alert">{{ error }}</div>

    <div class="projects">
      <div class="section-label">
        <span>{{ t('projects') }}</span>
        <span v-if="searching" role="status">{{ t('searching') }}</span>
        <span v-else-if="query.trim()">{{ t('matches', { count: hits.length }) }}</span>
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
        @alias="emit('alias', $event[0], $event[1], $event[2])"
      />

      <div v-if="!visibleProjects.length && !scanning && !searching" class="empty-mini">
        {{ query.trim() || providerId ? t('noMatches') : t('noSessionsFound') }}
      </div>

      <div v-if="mutationError" class="nav-notice error" role="alert">{{ mutationError }}</div>
      <HiddenSessionsPanel
        :sessions="visibleHiddenSessions"
        :selected-id="selectedId"
        :loading="hiddenLoading"
        :error="hiddenError"
        :busy-id="mutationLoading"
        :active-continuation-session-id="activeContinuationSessionId ?? null"
        :active-continuation-target-session-id="activeContinuationTargetSessionId ?? null"
        @select="emit('select', $event)"
        @restore="emit('restore', $event)"
      />
    </div>

    <div class="nav-footer">
      <button class="refresh-button" :disabled="scanning || continuationActive" :aria-describedby="continuationActive ? 'refresh-disabled-reason' : undefined" type="button" @click="emit('refresh')">
        {{ scanning ? t('scanning') : continuationActive ? t('refreshUnavailable') : t('refreshIndex') }}
      </button>
      <span v-if="continuationActive" id="refresh-disabled-reason" class="refresh-disabled-reason" role="status">{{ t('continuationInProgress') }}</span>
      <button class="settings-button" type="button" :aria-label="t('openSettings')" @click="emit('settings')">⚙</button>
      <span>{{ t('localIndexSourceUntouched') }}</span>
    </div>
  </aside>
</template>
