import {
  onScopeDispose,
  shallowReadonly,
  shallowRef,
  toValue,
  watch,
  type MaybeRefOrGetter,
} from 'vue';
import type { ScanTrigger } from '../../types';
import type { SessionBrowserRefreshResult } from '../browser/useSessionBrowser';

export interface ScanScheduleOptions {
  intervalSeconds?: MaybeRefOrGetter<number>;
  scanning?: MaybeRefOrGetter<boolean>;
  continuationActive?: MaybeRefOrGetter<boolean>;
  refresh: (trigger: ScanTrigger) => Promise<SessionBrowserRefreshResult>;
  now?: () => number;
}

export type ScanSkipReason = 'scanning' | 'continuation' | 'skipped_lifecycle' | null;

/** Local, lifecycle-aware timer. Browser scan leases serialize busy ticks. */
export function useScanSchedule(options: ScanScheduleOptions) {
  const intervalSeconds = shallowRef(Math.max(0, Math.floor(toValue(options.intervalSeconds ?? 0))));
  const enabled = shallowRef(intervalSeconds.value > 0);
  const nextRunAt = shallowRef<number | null>(null);
  const skipReason = shallowRef<ScanSkipReason>(null);
  const error = shallowRef<string | null>(null);
  let timer: number | undefined;
  let generation = 0;
  let disposed = false;

  const now = options.now ?? (() => Date.now());
  const clearTimer = () => {
    if (timer !== undefined) window.clearTimeout(timer);
    timer = undefined;
    nextRunAt.value = null;
  };

  function setNext(token: number) {
    if (disposed || token !== generation || intervalSeconds.value <= 0) return;
    const delay = intervalSeconds.value * 1000;
    nextRunAt.value = now() + delay;
    timer = window.setTimeout(() => void tick(token), delay);
  }

  async function tick(token: number) {
    timer = undefined;
    nextRunAt.value = null;
    if (disposed || token !== generation || intervalSeconds.value <= 0) return;
    skipReason.value = null;
    error.value = null;
    try {
      const result = await options.refresh('scheduled');
      if (disposed || token !== generation) return;
      if (result.status === 'skipped') skipReason.value = result.reason === 'skipped_lifecycle' ? 'skipped_lifecycle' : 'scanning';
      else if (result.status === 'error') error.value = result.error;
    } catch (cause) {
      if (disposed || token !== generation) return;
      error.value = cause instanceof Error ? cause.message : String(cause);
    } finally {
      if (!disposed && token === generation) setNext(token);
    }
  }

  function reschedule(value = intervalSeconds.value) {
    generation += 1;
    clearTimer();
    intervalSeconds.value = Math.max(0, Math.floor(value));
    enabled.value = intervalSeconds.value > 0;
    skipReason.value = null;
    error.value = null;
    if (enabled.value) setNext(generation);
  }

  function start() { reschedule(intervalSeconds.value); }
  function stop() { reschedule(0); }

  const stopWatch = watch(
    () => Number(toValue(options.intervalSeconds ?? intervalSeconds.value)),
    (value) => { if (value !== intervalSeconds.value) reschedule(value); },
  );
  const stopGateWatch = watch(
    [() => Boolean(toValue(options.scanning ?? false)), () => Boolean(toValue(options.continuationActive ?? false))],
    ([scanning, continuation]) => {
      if (!scanning && !continuation) skipReason.value = null;
    },
  );
  if (intervalSeconds.value > 0) setNext(generation);

  onScopeDispose(() => {
    disposed = true;
    generation += 1;
    clearTimer();
    stopWatch();
    stopGateWatch();
  });

  return {
    intervalSeconds: shallowReadonly(intervalSeconds),
    enabled: shallowReadonly(enabled),
    nextRunAt: shallowReadonly(nextRunAt),
    skipReason: shallowReadonly(skipReason),
    error: shallowReadonly(error),
    reschedule,
    start,
    stop,
  };
}
