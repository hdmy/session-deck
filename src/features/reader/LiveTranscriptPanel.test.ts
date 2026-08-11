import { createApp, nextTick } from 'vue';
import { afterEach, describe, expect, it } from 'vitest';
import LiveTranscriptPanel from './LiveTranscriptPanel.vue';

const roots: HTMLElement[] = [];

function mount(props: Record<string, unknown>) {
  const root = document.createElement('div');
  document.body.append(root);
  roots.push(root);
  createApp(LiveTranscriptPanel, props).mount(root);
  return root;
}

afterEach(() => {
  for (const root of roots.splice(0)) root.remove();
});

describe('LiveTranscriptPanel', () => {
  it('keeps tool and thinking events collapsed by default', async () => {
    const root = mount({
      sessionId: 'session-1', activeSessionId: 'session-1', continuationPhase: 'running',
      events: [
        { id: 'tool-1', kind: 'tool_use', role: null, content: 'run', timestamp: null, tool_name: 'rg', collapsed: true },
        { id: 'thinking-1', kind: 'thinking', role: null, content: 'plan', timestamp: null, tool_name: null, collapsed: true },
      ],
      tailPartial: false, tailDiagnostics: 0, tailError: null,
    });
    await nextTick();
    expect(root.querySelectorAll('details[open]')).toHaveLength(0);
    expect(root.textContent).toContain('Running');
  });

  it('shows no-new-event, partial, diagnostics, and tail error states', async () => {
    const root = mount({
      sessionId: 'session-1', activeSessionId: 'session-1', continuationPhase: 'draining', events: [],
      tailPartial: true, tailDiagnostics: 2, tailError: 'tail unavailable', continuationError: null,
    });
    await nextTick();
    expect(root.textContent).toContain('Draining');
    expect(root.textContent).toContain('finishing transcript');
    expect(root.textContent).toContain('is partial');
    expect(root.textContent).toContain('2 diagnostics');
    expect(root.textContent).toContain('tail unavailable');
  });
});
