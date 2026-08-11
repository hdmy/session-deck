<script setup lang="ts">
import type { TimelineEvent as Event } from '../../types';
import MarkdownContent from './MarkdownContent.vue';
import HighlightedText from './HighlightedText.vue';
import { formatTime, useI18n } from '../../i18n';

const props = defineProps<{ event: Event; finalResponse?: boolean; query?: string }>();

const { t } = useI18n();

function label(kind: Event['kind']): string {
  return ({
    user: t('you'),
    assistant: t('assistant'),
    thinking: t('thinking'),
    tool_use: t('toolCall'),
    tool_result: t('toolResult'),
    system: t('system'),
    unknown: t('event'),
  })[kind];
}

function time(value: number | null): string {
  return formatTime(value);
}
</script>

<template>
  <article
    :id="`event-${event.id}`"
    class="timeline-event"
    :class="[`kind-${event.kind}`, { 'is-collapsible': event.collapsed }]"
  >
    <div class="event-rail" aria-hidden="true"><span class="event-node"></span></div>
    <div class="event-body">
      <details v-if="event.collapsed" class="collapsed-event">
        <summary>
          <span class="event-kind">{{ label(event.kind) }}</span>
          <span class="collapsed-event-disclosure" aria-hidden="true"></span>
          <span v-if="event.tool_name" class="tool-name">{{ event.tool_name }}</span>
          <time v-if="event.timestamp">{{ time(event.timestamp) }}</time>
        </summary>
        <MarkdownContent v-if="props.finalResponse && event.content" :content="event.content" :query="props.query" />
        <pre v-else-if="event.content" class="event-content"><HighlightedText :text="event.content" :query="props.query" /></pre>
      </details>

      <template v-else>
        <header>
          <span class="event-kind">{{ label(event.kind) }}</span>
          <span v-if="event.tool_name" class="tool-name">{{ event.tool_name }}</span>
          <time v-if="event.timestamp">{{ time(event.timestamp) }}</time>
        </header>
        <MarkdownContent v-if="props.finalResponse && event.content" :content="event.content" :query="props.query" />
        <pre v-else-if="event.content" class="event-content"><HighlightedText :text="event.content" :query="props.query" /></pre>
      </template>
    </div>
  </article>
</template>
