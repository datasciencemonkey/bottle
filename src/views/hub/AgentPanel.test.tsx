// @vitest-environment jsdom
/** Slice 2a: the hub Agent panel. The view draws `hub:agent` and sends `AgentAction`s. Fake values. */
import { act } from 'react';
import { HUB_AGENT_EVENT, HUB_SCOPE, type HubAgentView } from '../../shared/hub-agent';
import type { AgentCallView, AgentTurnView } from '../../shared/ipc';
import { inputEmits, render, tauri, viewEmits } from '../test-tauri';
import { AgentPanel, NO_SESSION_COPY, useHubAgent } from './AgentPanel';
import { Hub } from './Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const T = (c: string) => c.repeat(32);
const sales = { name: 'Sales', colorVariant: 'sky' };
const fin = { name: 'Finance', colorVariant: 'rose' };
const call = (n: number, state: AgentCallView['state'] = 'ready'): AgentCallView => ({
  callId: `c-0000000${n}`,
  workspace: n === 1 ? sales : fin,
  question: `Question ${n}?`,
  state,
});
const turn = (c: string, o: Partial<AgentTurnView> = {}): AgentTurnView => ({
  turnId: T(c),
  question: `Compare ${c}?`,
  state: 'done',
  used: 2,
  budget: 10,
  agentWorkspace: sales,
  calls: [call(1), call(2)],
  parts: [],
  ...o,
});
const READY = {
  binary: 'ready',
  pinned: '0.5.0',
  version: '0.5.0',
  agentProfile: null,
  agentProfileState: 'ready',
} as const;
const NO_SESSION: HubAgentView['session'] = {
  state: 'none',
  agentProfile: null,
  turns: 0,
  queued: 0,
  tokens: 0,
};
const view = (o: Partial<HubAgentView> = {}): HubAgentView => ({
  session: NO_SESSION,
  waiting: [],
  earlier: [],
  genieCode: READY,
  ...o,
});
const busy = view({
  session: {
    state: 'busy',
    sessionId: T('d'),
    startedAt: 1_800_000_000_000,
    agentProfile: 'fake-sales',
    turns: 3,
    queued: 2,
    tokens: 124_000,
  },
  live: turn('a', { state: 'running', used: 1 }),
  waiting: [
    turn('b', { state: 'queued', position: 1, calls: [] }),
    turn('c', { state: 'queued', position: 2, calls: [] }),
  ],
  earlier: [
    turn('e', {
      tokens: 41_000,
      summary: { md: 'Finance is higher.', kind: 'note', workspaces: [fin] },
    }),
    turn('f'),
  ],
});
const actions = () => inputEmits().map((e) => (e as { payload: unknown }).payload);
const click = (el: Element | null | undefined) => act(async () => (el as HTMLElement).click());
const text = (el: HTMLElement) => (el.textContent ?? '').replace(/\s+/g, ' ');

beforeEach(() => tauri.reset());
afterEach(() => {
  document.body.innerHTML = '';
});

describe('AgentPanel', () => {
  it('[2a.1] no session: the fixed line, and New session is disabled', async () => {
    const { el } = await render(<AgentPanel view={view()} active />);
    expect(el.querySelector('[data-agent-empty]')?.textContent).toBe(NO_SESSION_COPY);
    expect(el.querySelector<HTMLButtonElement>('[data-agent-new-session]')?.disabled).toBe(true);
  });

  it('with Genie Code missing the empty state says why and offers Settings', async () => {
    const v = view({ genieCode: { ...READY, binary: 'not_installed' } });
    const { el } = await render(<AgentPanel view={v} active />);
    expect(el.querySelector('[data-agent-empty]')?.textContent).toContain('Not installed');
    await click(el.querySelector('[data-agent-open-settings]'));
    expect(actions()).toEqual([{ action: 'agent.open_settings' }]);
  });

  it('shows the header with the workspace, the count and the tokens', async () => {
    const { el } = await render(<AgentPanel view={busy} active />);
    const h = text(el.querySelector('[data-agent-header]') as HTMLElement);
    expect(h).toContain('Genie Code in Sales · started ');
    expect(h).toContain('3 questions · 124k tokens');
  });

  it('[AC-2a.9] the live turn shows its calls, "n of N" and a Stop button with a full name', async () => {
    const { el } = await render(<AgentPanel view={busy} active />);
    const live = el.querySelector('[data-agent-live]') as HTMLElement;
    expect(live.querySelectorAll('[data-agent-call]')).toHaveLength(2);
    expect(text(live)).toContain('1 of 10 Genie calls');
    const stop = live.querySelector('[data-agent-stop]');
    expect(stop?.getAttribute('aria-label')).toBe('Stop: Compare a?');
    await click(stop);
    expect(actions()).toEqual([{ action: 'agent.stop', turnId: T('a') }]);
  });

  it('[AC-2a.10] a slow turn shows "Still working…"', async () => {
    const v = view({ ...busy, live: turn('a', { state: 'slow' }) });
    const { el } = await render(<AgentPanel view={v} active />);
    expect(text(el.querySelector('[data-agent-live]') as HTMLElement)).toContain('Still working…');
  });

  it('[AC-2a.19] [AC-2a.21] waiting rows show the copy and a × that removes only that row', async () => {
    const { el } = await render(<AgentPanel view={busy} active />);
    const rows = el.querySelectorAll('[data-agent-waiting]');
    expect(rows).toHaveLength(2);
    expect(rows[0]?.textContent).toContain('Sends when the answer above lands');
    const x = el.querySelector(`[data-agent-remove="${T('c')}"]`);
    expect(x?.getAttribute('aria-label')).toBe('Remove from the queue: Compare c?');
    await click(x);
    expect(actions()).toEqual([{ action: 'agent.remove_queued', turnId: T('c') }]);
  });

  it('[AC-2a.20] never draws more than the queue holds: three waiting rows at most', async () => {
    const four = view({
      ...busy,
      waiting: [turn('1'), turn('2'), turn('3')].map((t) => ({ ...t, state: 'queued' as const })),
    });
    const { el } = await render(<AgentPanel view={four} active />);
    expect(el.querySelectorAll('[data-agent-waiting]')).toHaveLength(3);
  });

  it('shows the earlier turns in the order it gets them, with note, tokens and no Stop', async () => {
    const { el } = await render(<AgentPanel view={busy} active />);
    const rows = [...el.querySelectorAll('[data-agent-earlier]')];
    expect(rows.map((r) => r.getAttribute('data-agent-earlier'))).toEqual([T('e'), T('f')]);
    expect(rows[0]?.textContent).toContain('Note from your MOG');
    expect(rows[0]?.textContent).toContain('41k tokens');
    expect(rows[0]?.querySelector('[data-agent-stop]')).toBeNull();
  });

  it('New session sends its action and puts the focus on the heading', async () => {
    const { el } = await render(<AgentPanel view={busy} active />);
    await click(el.querySelector('[data-agent-new-session]'));
    expect(actions()).toEqual([{ action: 'agent.new_session' }]);
    await act(async () => new Promise((r) => requestAnimationFrame(() => r(undefined))));
    expect(document.activeElement).toBe(el.querySelector('[data-agent-header]'));
  });

  it('the controls are real buttons with names (keyboard and aria)', async () => {
    const { el } = await render(<AgentPanel view={busy} active />);
    for (const b of el.querySelectorAll<HTMLElement>(
      '[data-agent-stop], [data-agent-remove], [data-agent-new-session]',
    )) {
      expect(b.tagName).toBe('BUTTON');
      expect((b.getAttribute('aria-label') ?? b.textContent ?? '').trim()).not.toBe('');
    }
    expect(el.querySelector('[data-agent-header]')?.getAttribute('tabindex')).toBe('-1');
  });

  it('an error notice is an alert and an info notice is a polite status', async () => {
    const e = await render(
      <AgentPanel view={view({ notice: { kind: 'error', text: 'Bad.' } })} active />,
    );
    expect(e.el.querySelector('[role="alert"]')?.textContent).toBe('Bad.');
    const i = await render(
      <AgentPanel view={view({ notice: { kind: 'info', text: 'Fine.' } })} active />,
    );
    expect(i.el.querySelector('[data-agent-notice="info"]')?.textContent).toBe('Fine.');
  });

  it('is hidden when its area is not active', async () => {
    const { el } = await render(<AgentPanel view={busy} active={false} />);
    expect(el.querySelector<HTMLElement>('[data-hub-view="agent"]')?.hidden).toBe(true);
  });
});

describe('the hub with the Agent area', () => {
  function Host() {
    return <AgentPanel view={useHubAgent()} active />;
  }

  it('listens on the hub window only and sends agent.ready once', async () => {
    await render(<Host />);
    expect(tauri.listenOptions.get(HUB_AGENT_EVENT)).toEqual([{ target: 'hub' }]);
    expect(
      viewEmits().filter(
        (e) => (e as { payload?: { action?: string } }).payload?.action === 'agent.ready',
      ),
    ).toEqual([{ mogId: HUB_SCOPE, kind: 'click', payload: { action: 'agent.ready' } }]);
  });

  it('draws a valid panel and ignores a bad one', async () => {
    const { el } = await render(<Host />);
    await tauri.emitTo('hub', HUB_AGENT_EVENT, busy);
    expect(el.querySelector('[data-agent-live]')).not.toBeNull();
    await tauri.emitTo('hub', HUB_AGENT_EVENT, { nonsense: true });
    expect(el.querySelector('[data-agent-live]')).not.toBeNull();
  });

  it('shows the Agent area in the sidebar between Conversations and Recordings, and opens on a new notice', async () => {
    const { el } = await render(<Hub />);
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]')?.click());
    const ids = [...el.querySelectorAll('[data-sidebar-item]')].map((n) =>
      n.getAttribute('data-sidebar-item'),
    );
    expect(ids.indexOf('agent')).toBe(ids.indexOf('conversations') + 1);
    expect(ids.indexOf('recordings')).toBe(ids.indexOf('agent') + 1);
    await tauri.emitTo(
      'hub',
      HUB_AGENT_EVENT,
      view({ notice: { kind: 'info', text: 'Showing your Genie Code session.' } }),
    );
    expect(el.querySelector<HTMLElement>('[data-hub-view="agent"]')?.hidden).toBe(false);
  });
});
