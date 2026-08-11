<script setup lang="ts">
import '@xterm/xterm/css/xterm.css';
import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import { computed, onMounted, onUnmounted, shallowRef, useTemplateRef, watch } from 'vue';
import type { ContinuationViewState } from './continuationTypes';
import { MAX_TERMINAL_DOCK_HEIGHT, MIN_TERMINAL_DOCK_HEIGHT } from './terminalDockResize';
import { useContinuationTerminal } from './useContinuationTerminal';
import { useTerminalDockResize } from './useTerminalDockResize';
import { useI18n } from '../../i18n';

const props = defineProps<{ sessionId: string; title: string; mode?: 'resume' | 'fork' }>();
const emit = defineEmits<{
  closed: [reason: 'exited' | 'closed'];
  'state-change': [state: ContinuationViewState];
}>();
const host = useTemplateRef<HTMLDivElement>('terminal-host');
const minimized = shallowRef(false);
const terminal = shallowRef<Terminal | null>(null);
const fit = shallowRef<FitAddon | null>(null);
const { t } = useI18n();
let emittedClose = false;
let mounted = false;
let startFrame: number | undefined;
let fitFrame: number | undefined;

const controller = useContinuationTerminal({ onOutput: (data) => terminal.value?.write(new Uint8Array(data)), onFinished: (reason) => { if (!emittedClose) { emittedClose = true; emit('closed', reason); } } });
const phase = controller.phase;
const terminalError = controller.error;
const tailError = controller.tailError;
const tailPartial = controller.tailPartial;
const tailDiagnostics = controller.tailDiagnostics;
const phaseLabel = computed(() => ({ idle: t('ready'), preflighting: t('preflighting'), starting: t('starting'), running: t('running'), draining: t('drainingTranscript'), exited: t('exited'), error: t('error'), closed: t('closed') })[phase.value]);

function emitState() {
  emit('state-change', {
    // The runtime may replace the parent with a newly-created child during a
    // fork. Never report the prop (which is always the parent) as the active
    // runtime session once the controller has an authoritative ID.
    sessionId: controller.sessionId?.value ?? '',
    parentSessionId: controller.parentSessionId?.value ?? null,
    mode: controller.mode?.value ?? (props.mode ?? 'resume'),
    phase: phase.value,
    status: controller.status.value,
    liveEvents: controller.liveEvents.value,
    noNewEvents: controller.noNewEvents.value,
    tailPartial: controller.tailPartial.value,
    tailDiagnostics: controller.tailDiagnostics.value,
    tailError: controller.tailError.value,
    tailCaughtUp: controller.tailCaughtUp?.value ?? false,
    error: terminalError.value,
  });
}

watch(
  () => [phase.value, controller.status.value, controller.sessionId?.value, controller.parentSessionId?.value, controller.mode?.value, controller.liveEvents.value, controller.noNewEvents.value, controller.tailPartial.value, controller.tailDiagnostics.value, controller.tailError.value, controller.tailCaughtUp?.value, terminalError.value],
  emitState,
  { deep: true, immediate: true, flush: 'sync' },
);

function fitTerminal() { if (!mounted || !terminal.value || !fit.value || minimized.value) return; fit.value.fit(); void controller.resize(terminal.value.rows, terminal.value.cols, host.value?.clientWidth ?? 0, host.value?.clientHeight ?? 0); }
function scheduleFit() {
  if (!mounted || fitFrame !== undefined) return;
  fitFrame = window.requestAnimationFrame(() => {
    fitFrame = undefined;
    if (mounted) fitTerminal();
  });
}
const { height, beginResize, finishResize, resizeWithKeyboard } = useTerminalDockResize({
  onResizeStart: () => { minimized.value = false; },
  onHeightChange: fitTerminal,
});
async function closeTerminal() {
  let result: Awaited<ReturnType<typeof controller.close>>;
  try {
    result = await controller.close();
  } catch {
    // A controller implementation may reject instead of returning the
    // discriminated failure. Keep the dock mounted and preserve guards.
    return;
  }
  // Only a backend-confirmed close (or an already terminal controller state)
  // may tear down the dock. A rejected close keeps this component mounted so
  // the user can retry and App-level guards remain intact.
  if (result?.status === 'closed' && !emittedClose) {
    emittedClose = true;
    emit('closed', 'closed');
  }
}
onMounted(() => {
  mounted = true;
  if (!host.value) return;
  const nextFit = new FitAddon(); const nextTerminal = new Terminal({ convertEol: true, cursorBlink: true, fontSize: 12, fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace', theme: { background: '#0b1014', foreground: '#d5dde3', cursor: '#75b9a4' } });
  nextTerminal.loadAddon(nextFit); nextTerminal.open(host.value); terminal.value = nextTerminal; fit.value = nextFit; nextTerminal.onData((data: string) => void controller.write(data));
  startFrame = window.requestAnimationFrame(() => {
    startFrame = undefined;
    // The dock can be removed before its first frame (for example, when the
    // user selects another session). Never start a continuation after that
    // component has been unmounted.
    if (!mounted) return;
    fitTerminal();
    void controller.start(props.sessionId, nextTerminal.rows, nextTerminal.cols, props.mode ?? 'resume');
  });
});
watch(minimized, scheduleFit);
onUnmounted(() => {
  mounted = false;
  if (startFrame !== undefined) window.cancelAnimationFrame(startFrame);
  if (fitFrame !== undefined) window.cancelAnimationFrame(fitFrame);
  startFrame = undefined;
  fitFrame = undefined;
  terminal.value?.dispose();
});
</script>

<template>
  <section class="continuation-dock" :style="{ height: minimized ? '48px' : `${height}px` }" :aria-label="t('claudeTerminal')">
    <div class="terminal-resize-handle" role="separator" :aria-label="t('resizeTerminal')" aria-orientation="horizontal" :aria-valuemin="MIN_TERMINAL_DOCK_HEIGHT" :aria-valuemax="MAX_TERMINAL_DOCK_HEIGHT" :aria-valuenow="height" tabindex="0" @pointerdown="beginResize" @pointerup="finishResize" @pointercancel="finishResize" @lostpointercapture="finishResize" @keydown="resizeWithKeyboard"></div>
    <header class="terminal-head"><div><strong>{{ props.mode === 'fork' ? t('claudeForkTerminal') : t('claudeTerminal') }}</strong><span>{{ title }} · {{ t('runtimeOutput') }}</span><span v-if="props.mode === 'fork'" class="fork-notice" role="status">{{ t('forkPtyNotice') }}</span><span class="terminal-status" role="status" aria-live="polite">{{ phaseLabel }}</span></div><div class="terminal-controls"><button type="button" @click="minimized = !minimized">{{ minimized ? t('expand') : t('minimize') }}</button><button type="button" @click="closeTerminal">{{ t('close') }}</button></div></header>
    <div v-if="!minimized" ref="terminal-host" class="terminal-host"></div>
    <div v-if="phase === 'starting' && !minimized" class="terminal-overlay">{{ t('terminalStarting') }}</div>
    <div v-if="terminalError && !minimized" class="terminal-error" role="alert">{{ terminalError }}</div>
    <div v-if="tailError && !minimized" class="terminal-error tail-error" role="alert">{{ t('liveTextUnavailable', { value: tailError }) }}</div>
    <div v-if="!minimized && (tailPartial || tailDiagnostics)" class="terminal-tail-notice" role="status">
      {{ t('liveTranscriptPartial', { suffix: tailPartial ? t('partialSuffix') : '' }) }}<span v-if="tailDiagnostics"> · {{ t('diagnostics', { count: tailDiagnostics }) }}</span>
    </div>
  </section>
</template>

<style scoped>
.continuation-dock { position: relative; flex: none; min-height: 48px; background: #0b1014; box-shadow: 0 -12px 28px #0004; }
.terminal-resize-handle { position: relative; display: block; height: 6px; cursor: row-resize; touch-action: none; outline: 0; }
.terminal-resize-handle::after { position: absolute; top: 2px; right: 0; left: 0; border-top: 1px solid #33434a; content: ''; transition: border-color .12s ease, box-shadow .12s ease; }
.terminal-resize-handle:hover::after, .terminal-resize-handle:focus-visible::after { border-color: #75b9a4; box-shadow: 0 0 0 1px #75b9a455; }
.terminal-head { height: 42px; display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 0 14px; border-bottom: 1px solid #253239; }
.terminal-head > div:first-child { display: flex; align-items: baseline; gap: 10px; min-width: 0; }
.terminal-head strong { color: #c9e8dd; font-size: 12px; }
.terminal-head span { overflow: hidden; color: #71818a; font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }
.terminal-head .terminal-status { flex: none; color: #c9e8dd; }
.terminal-head .fork-notice { flex: none; color: #c7a86c; }
.terminal-controls { display: flex; align-items: center; gap: 7px; }
.terminal-controls button { border: 1px solid #35464d; border-radius: 4px; padding: 4px 7px; background: transparent; color: #9db0b3; cursor: pointer; font: inherit; font-size: 10px; }
.terminal-controls button:hover, .terminal-controls button:focus-visible { border-color: #75b9a4; color: #c9e8dd; outline: 0; }
.terminal-host { height: calc(100% - 48px); padding: 8px 10px; }
.terminal-overlay { position: absolute; inset: 48px 0 0; display: grid; place-items: center; color: #8ba0a0; background: #0b1014cc; font-size: 12px; }
.terminal-error { position: absolute; right: 12px; bottom: 10px; max-width: 60%; padding: 6px 8px; border: 1px solid #8c4f4f; border-radius: 4px; background: #4d2424e8; color: #efb1b1; font-size: 11px; }
.terminal-tail-notice { position: absolute; right: 12px; bottom: 10px; max-width: 60%; padding: 6px 8px; border: 1px solid #806b42; border-radius: 4px; background: #473b22e8; color: #e0c997; font-size: 11px; }
.tail-error { bottom: 42px; }
</style>
