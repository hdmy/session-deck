<script setup lang="ts">
import type { TimelineEvent as Event } from '../../types';
import MarkdownContent from './MarkdownContent.vue';
import HighlightedText from './HighlightedText.vue';

const props = defineProps<{ event: Event; finalResponse?: boolean; query?: string }>();

const labels: Record<Event['kind'], string> = {
  user: 'You',
  assistant: 'Assistant',
  thinking: 'Thinking',
  tool_use: 'Tool call',
  tool_result: 'Tool result',
  system: 'System',
  unknown: 'Event',
};

function time(value: number | null): string {
  return value
    ? new Date(value).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
    : '';
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
          <span class="event-kind">{{ labels[event.kind] }}</span>
          <span class="collapsed-event-disclosure" aria-hidden="true"></span>
          <span v-if="event.tool_name" class="tool-name">{{ event.tool_name }}</span>
          <time v-if="event.timestamp">{{ time(event.timestamp) }}</time>
        </summary>
        <MarkdownContent v-if="props.finalResponse && event.content" :content="event.content" :query="props.query" />
        <pre v-else-if="event.content" class="event-content"><HighlightedText :text="event.content" :query="props.query" /></pre>
      </details>

      <template v-else>
        <header>
          <span class="event-kind">{{ labels[event.kind] }}</span>
          <span v-if="event.tool_name" class="tool-name">{{ event.tool_name }}</span>
          <time v-if="event.timestamp">{{ time(event.timestamp) }}</time>
        </header>
        <MarkdownContent v-if="props.finalResponse && event.content" :content="event.content" :query="props.query" />
        <pre v-else-if="event.content" class="event-content"><HighlightedText :text="event.content" :query="props.query" /></pre>
      </template>
    </div>
  </article>
</template>
