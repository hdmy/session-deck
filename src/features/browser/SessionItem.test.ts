import { createApp, defineComponent, h, nextTick, shallowRef } from 'vue';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SessionSummary } from '../../types';
import SessionItem from './SessionItem.vue';

const roots: { element: HTMLElement; app: ReturnType<typeof createApp> }[] = [];

function session(): SessionSummary {
  return {
    id: 'session-1', native_session_id: 'native-1', provider_id: 'claude', project_id: 'project-1', title: 'Session',
    source_title: 'Session', hidden: false, pinned: false, last_used_at: null, started_at: 1,
    ended_at: null, branch: 'main', first_prompt: 'hello', last_prompt: 'hello', cwd: '/tmp/project',
    models: ['claude'], tool_count: 0, source_mtime: 1, partial: false,
  };
}

function mount(props: Record<string, unknown>) {
  const element = document.createElement('div');
  document.body.append(element);
  const app = createApp(SessionItem, props);
  app.mount(element);
  roots.push({ element, app });
  return element;
}

afterEach(() => {
  for (const { element, app } of roots.splice(0)) {
    app.unmount();
    element.remove();
  }
});

describe('SessionItem continuation locking', () => {
  it('keeps selection available while locking active-session management', async () => {
    const onSelect = vi.fn();
    const root = mount({ session: session(), active: true, continuationActive: true, onSelect });
    await nextTick();
    expect(root.querySelector<HTMLButtonElement>('.session-item')?.disabled).toBe(false);
    expect(root.querySelector<HTMLButtonElement>('.hide-action')?.disabled).toBe(true);
    root.querySelector<HTMLButtonElement>('.session-item')?.click();
    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  it('guards Enter submit when continuation starts after editing began', async () => {
    const continuationActive = shallowRef(false);
    const onRename = vi.fn();
    const root = document.createElement('div');
    document.body.append(root);
    const app = createApp(defineComponent({
      setup: () => () => h(SessionItem, {
        session: session(), active: true, continuationActive: continuationActive.value, onRename,
      }),
    }));
    app.mount(root);
    roots.push({ element: root, app });
    await nextTick();
    root.querySelector<HTMLButtonElement>('[aria-label="Rename session"]')?.click();
    await nextTick();
    continuationActive.value = true;
    await nextTick();
    const form = root.querySelector('form');
    expect(form).not.toBeNull();
    form?.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    expect(onRename).not.toHaveBeenCalled();
    expect(root.querySelector<HTMLButtonElement>('[aria-label="Save title"]')?.disabled).toBe(true);
    expect(root.querySelector<HTMLButtonElement>('[aria-label="Cancel rename"]')?.getAttribute('aria-describedby')).toBeTruthy();
  });

  it('does not submit while an IME composition is being committed', async () => {
    const onRename = vi.fn();
    const root = mount({ session: session(), active: true, onRename });
    await nextTick();
    root.querySelector<HTMLButtonElement>('[aria-label="Rename session"]')?.click();
    await nextTick();
    const input = root.querySelector<HTMLInputElement>('.session-rename-input');
    input?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', keyCode: 229, bubbles: true, isComposing: true }));
    expect(onRename).not.toHaveBeenCalled();
    expect(root.querySelector('form')).not.toBeNull();
  });
});
