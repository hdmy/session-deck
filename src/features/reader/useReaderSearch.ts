import { computed, nextTick, type Ref, shallowRef, watch } from 'vue';
import type { SessionDetail } from '../../types';
import {
  displayedEvents,
  findReaderMatches,
  nextMatchIndex,
  previousMatchIndex,
  scrollToReaderMatch,
  searchReaderEvents,
  type ReaderSearchMatch,
} from './readerSearch';

export interface UseReaderSearchOptions {
  detail: Ref<SessionDetail | null>;
  mode: Ref<'focus' | 'full'>;
  onNeedsFull?: () => void;
}

export function useReaderSearch(options: UseReaderSearchOptions) {
  const query = shallowRef('');
  const currentIndex = shallowRef(-1);
  const matches = computed(() => {
    const detail = options.detail.value;
    if (!detail || !query.value.trim()) return [];
    return searchReaderEvents(displayedEvents(detail, options.mode.value), query.value);
  });
  const fullMatches = computed(() => {
    const detail = options.detail.value;
    if (!detail || !query.value.trim()) return [];
    return searchReaderEvents(displayedEvents(detail, 'full'), query.value);
  });
  const currentMatch = computed(() => matches.value[currentIndex.value]);
  const matchCount = computed(() => matches.value.length);
  const hasFullOnlyMatch = computed(() => options.mode.value === 'focus' && matches.value.length === 0 && fullMatches.value.length > 0);
  const status = computed(() => {
    if (!query.value.trim()) return 'idle' as const;
    if (!matches.value.length && !hasFullOnlyMatch.value) return 'none' as const;
    return 'matches' as const;
  });

  function setQuery(value: string) {
    query.value = value;
    // A new query has no located match yet. Next/Enter must land on the first
    // result rather than skipping it.
    currentIndex.value = -1;
  }

  function locate(index: number): boolean {
    if (index < 0 || index >= matches.value.length) return false;
    currentIndex.value = index;
    return scrollToReaderMatch(matches.value[index]);
  }

  function next() {
    if (hasFullOnlyMatch.value) {
      options.onNeedsFull?.();
      // The full-mode events are rendered on the next Vue flush. Deferring the
      // first locate keeps the first button press useful instead of requiring
      // users to click again after switching views.
      void nextTick(() => {
        if (options.mode.value === 'full') locate(0);
      });
      return true;
    }
    const sourceCount = matches.value.length;
    if (!sourceCount) return false;
    return locate(nextMatchIndex(currentIndex.value, sourceCount));
  }

  function previous() {
    if (hasFullOnlyMatch.value) {
      options.onNeedsFull?.();
      void nextTick(() => {
        if (options.mode.value === 'full') locate(0);
      });
      return true;
    }
    const sourceCount = matches.value.length;
    if (!sourceCount) return false;
    return locate(previousMatchIndex(currentIndex.value, sourceCount));
  }

  function clear() {
    query.value = '';
    currentIndex.value = -1;
  }

  watch([() => options.detail.value?.summary.id, () => options.detail.value?.selected_branch_id, () => options.mode.value], () => {
    currentIndex.value = -1;
  });
  watch(matches, (next) => {
    if (next.length === 0) currentIndex.value = -1;
    else if (currentIndex.value >= next.length) currentIndex.value = 0;
  });

  return {
    query,
    matches,
    currentMatch,
    currentIndex,
    matchCount,
    fullMatchCount: computed(() => fullMatches.value.length),
    hasFullOnlyMatch,
    status,
    setQuery,
    next,
    previous,
    locate,
    clear,
  };
}

export { findReaderMatches };
export type { ReaderSearchMatch };
