import { onScopeDispose, shallowReadonly, shallowRef } from 'vue';
import {
  clampTerminalDockHeight,
  dockHeightFromKey,
  dockHeightFromPointer,
  MAX_TERMINAL_DOCK_HEIGHT,
  MIN_TERMINAL_DOCK_HEIGHT,
} from './terminalDockResize';

interface TerminalDockResizeOptions {
  initialHeight?: number;
  minHeight?: number;
  maxHeight?: number;
  onResizeStart?: () => void;
  onHeightChange?: () => void;
}

export function useTerminalDockResize(options: TerminalDockResizeOptions = {}) {
  const minHeight = options.minHeight ?? MIN_TERMINAL_DOCK_HEIGHT;
  const maxHeight = options.maxHeight ?? MAX_TERMINAL_DOCK_HEIGHT;
  const height = shallowRef(clampTerminalDockHeight(options.initialHeight ?? 280, minHeight, maxHeight));
  let activePointerId: number | null = null;
  let startY = 0;
  let startHeight = height.value;
  let animationFrame: number | null = null;

  function notifyHeightChange() {
    if (animationFrame !== null) return;
    animationFrame = window.requestAnimationFrame(() => {
      animationFrame = null;
      options.onHeightChange?.();
    });
  }

  function setHeight(nextHeight: number) {
    const next = clampTerminalDockHeight(nextHeight, minHeight, maxHeight);
    if (next === height.value) return;
    height.value = next;
    notifyHeightChange();
  }

  function finishResize(event?: PointerEvent) {
    if (event && event.pointerId !== activePointerId) return;
    activePointerId = null;
    window.removeEventListener('pointermove', moveResize);
    window.removeEventListener('pointerup', finishResize);
    window.removeEventListener('pointercancel', finishResize);
  }

  function moveResize(event: PointerEvent) {
    if (event.pointerId !== activePointerId) return;
    setHeight(dockHeightFromPointer(startHeight, startY, event.clientY, minHeight, maxHeight));
  }

  function beginResize(event: PointerEvent) {
    if (event.button !== 0) return;
    event.preventDefault();
    options.onResizeStart?.();
    activePointerId = event.pointerId;
    startY = event.clientY;
    startHeight = height.value;
    (event.currentTarget as HTMLElement | null)?.setPointerCapture?.(event.pointerId);
    window.addEventListener('pointermove', moveResize);
    window.addEventListener('pointerup', finishResize);
    window.addEventListener('pointercancel', finishResize);
  }

  function resizeWithKeyboard(event: KeyboardEvent) {
    const next = dockHeightFromKey(height.value, event.key, minHeight, maxHeight);
    if (next === null) return;
    event.preventDefault();
    options.onResizeStart?.();
    setHeight(next);
  }

  onScopeDispose(() => {
    finishResize();
    if (animationFrame !== null) window.cancelAnimationFrame(animationFrame);
  });

  return {
    height: shallowReadonly(height),
    beginResize,
    finishResize,
    resizeWithKeyboard,
  };
}
