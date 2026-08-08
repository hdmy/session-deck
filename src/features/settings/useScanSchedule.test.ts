import { effectScope, shallowRef } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useScanSchedule } from './useScanSchedule';

describe('useScanSchedule', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('runs a scheduled refresh and keeps the next timer', async () => {
    const refresh = vi.fn().mockResolvedValue({ status: 'success', report: { partial: false } });
    const scope = effectScope();
    const interval = shallowRef(60);
    let schedule!: ReturnType<typeof useScanSchedule>;
    scope.run(() => { schedule = useScanSchedule({ intervalSeconds: interval, refresh }); });
    await vi.advanceTimersByTimeAsync(60_000);
    expect(refresh).toHaveBeenCalledWith('scheduled');
    expect(schedule.nextRunAt.value).not.toBeNull();
    scope.stop();
  });

  it('records a lifecycle-busy tick through the scheduled backend scan', async () => {
    const refresh = vi.fn().mockResolvedValue({ status: 'skipped', reason: 'skipped_lifecycle' });
    const scanning = shallowRef(true);
    const scope = effectScope();
    let schedule!: ReturnType<typeof useScanSchedule>;
    scope.run(() => { schedule = useScanSchedule({ intervalSeconds: 60, scanning, refresh }); });
    await vi.advanceTimersByTimeAsync(60_000);
    expect(refresh).toHaveBeenCalledWith('scheduled');
    expect(schedule.skipReason.value).toBe('skipped_lifecycle');
    scope.stop();
  });

  it('reconfigures and disables the old timer', async () => {
    const refresh = vi.fn().mockResolvedValue({ status: 'success', report: { partial: false } });
    const scope = effectScope();
    let schedule!: ReturnType<typeof useScanSchedule>;
    scope.run(() => { schedule = useScanSchedule({ intervalSeconds: 60, refresh }); });
    schedule.reschedule(0);
    await vi.advanceTimersByTimeAsync(120_000);
    expect(refresh).not.toHaveBeenCalled();
    expect(schedule.enabled.value).toBe(false);
    scope.stop();
  });

  it('ignores a late refresh result after reconfiguration', async () => {
    let resolve!: (result: any) => void;
    const refresh = vi.fn().mockReturnValue(new Promise((done) => { resolve = done; }));
    const scope = effectScope();
    let schedule!: ReturnType<typeof useScanSchedule>;
    scope.run(() => { schedule = useScanSchedule({ intervalSeconds: 60, refresh }); });
    const tick = vi.advanceTimersByTimeAsync(60_000);
    schedule.reschedule(0);
    resolve({ status: 'success', report: { partial: false } });
    await tick;
    expect(schedule.nextRunAt.value).toBeNull();
    scope.stop();
  });
});
