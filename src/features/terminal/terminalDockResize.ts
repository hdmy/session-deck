export const MIN_TERMINAL_DOCK_HEIGHT = 180;
export const MAX_TERMINAL_DOCK_HEIGHT = 620;
const KEYBOARD_RESIZE_STEP = 24;

export function clampTerminalDockHeight(
  value: number,
  minHeight = MIN_TERMINAL_DOCK_HEIGHT,
  maxHeight = MAX_TERMINAL_DOCK_HEIGHT,
): number {
  return Math.min(maxHeight, Math.max(minHeight, Math.round(value)));
}

export function dockHeightFromPointer(
  startHeight: number,
  startY: number,
  currentY: number,
  minHeight = MIN_TERMINAL_DOCK_HEIGHT,
  maxHeight = MAX_TERMINAL_DOCK_HEIGHT,
): number {
  return clampTerminalDockHeight(startHeight + startY - currentY, minHeight, maxHeight);
}

export function dockHeightFromKey(
  height: number,
  key: string,
  minHeight = MIN_TERMINAL_DOCK_HEIGHT,
  maxHeight = MAX_TERMINAL_DOCK_HEIGHT,
): number | null {
  if (key === 'ArrowUp') return clampTerminalDockHeight(height + KEYBOARD_RESIZE_STEP, minHeight, maxHeight);
  if (key === 'ArrowDown') return clampTerminalDockHeight(height - KEYBOARD_RESIZE_STEP, minHeight, maxHeight);
  if (key === 'Home') return minHeight;
  if (key === 'End') return maxHeight;
  return null;
}
