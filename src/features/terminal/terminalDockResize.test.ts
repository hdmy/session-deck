import { describe, expect, it } from 'vitest';
import {
  clampTerminalDockHeight,
  dockHeightFromKey,
  dockHeightFromPointer,
  MAX_TERMINAL_DOCK_HEIGHT,
  MIN_TERMINAL_DOCK_HEIGHT,
} from './terminalDockResize';

describe('terminal dock resize helpers', () => {
  it('grows upward and clamps to the permitted dock range', () => {
    expect(dockHeightFromPointer(280, 500, 440)).toBe(340);
    expect(dockHeightFromPointer(280, 500, 900)).toBe(MIN_TERMINAL_DOCK_HEIGHT);
    expect(dockHeightFromPointer(600, 500, 100)).toBe(MAX_TERMINAL_DOCK_HEIGHT);
    expect(clampTerminalDockHeight(280.6)).toBe(281);
  });

  it('supports keyboard resizing for an accessible splitter', () => {
    expect(dockHeightFromKey(280, 'ArrowUp')).toBe(304);
    expect(dockHeightFromKey(280, 'ArrowDown')).toBe(256);
    expect(dockHeightFromKey(280, 'Home')).toBe(MIN_TERMINAL_DOCK_HEIGHT);
    expect(dockHeightFromKey(280, 'End')).toBe(MAX_TERMINAL_DOCK_HEIGHT);
    expect(dockHeightFromKey(280, 'Enter')).toBeNull();
  });
});
