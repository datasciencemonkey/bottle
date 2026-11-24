// @vitest-environment jsdom
/**
 * Agent threads in the hub Conversations area (the MOG agent in Conversations): the list (In
 * progress, finished rows, Clear failed), the thread detail (header, turns, live turn, queue),
 * Delete and Undo, and the composer. The view draws `hub:conversations` and `hub:agent` and sends
 * actions with `view_emit` only (ARC-4). All values are plainly fake.
 */
import { act } from 'react';
import type { Root } from 'react-dom/client';
import { HUB_AGENT_EVENT, HUB_SCOPE, type HubAgentView } from '../../shared/hub-agent';
import {
  CONVERSATIONS_VIEW_ID,
  HUB_CONVERSATIONS_EVENT,
  HUB_CONVERSATIONS_OPEN_EVENT,
  type AgentThread,
  type AgentThreadTurn,
  type ConversationRow,
} from '../../shared/hub-conversations';
import { MEMORY_UNDO_MS } from '../../shared/hub-memory';
import type { AgentCallView, AgentTurnView } from '../../shared/ipc';
import { render, tauri, viewEmits } from '../test-tauri';
import { Conversations } from './Conversations';
import { Hub } from './Hub';
import { useHubAgent } from './use-hub-agent';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const NOW = new Date(2026, 10, 24, 16, 30).getTime();
const at = (h: number, m: number) => new Date(2026, 10, 24, h, m).getTime();
const MOG = '00000000-0000-4000-8000-00000000000a';
const T = (c: string) => c.repeat(32);
const uuidOf = (t: string) =>
  `${t.slice(0, 8)}-${t.slice(8, 12)}-${t.slice(12, 16)}-${t.slice(16, 20)}-${t.slice(20)}`;
const THREAD_A = '51111111-2222-4333-8444-00000000000a';
const THREAD_B = '51111111-2222-4333-8444-00000000000b';
const THREAD_C = '51111111-2222-4333-8444-00000000000c';
const sales = { name: 'Sales', colorVariant: 'sky' };
const fin = { name: 'Finance', colorVariant: 'rose' };

const call = (n: number, state: AgentCallView['state'] = 'ready'): AgentCallView => ({
  callId: `c-0000000${n}`,
  workspace: n === 1 ? sales : fin,
  question: `Question ${n}?`,
  state,
});
const memTurn = (c: string, o: Partial<AgentTurnView> = {}): AgentTurnView => ({
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
const part = (n: number, o: Record<string, unknown> = {}) => ({
  requestId: `62222222-3333-4444-8555-00000000000${n}`,
  workspace: n === 1 ? sales : fin,
  profile: `fake-${n}`,
  workspaceHost: 'example.cloud.databricks.test',
  status: 'answered' as const,
  answerMd: `Answer **${n}** from a workspace.`,
  keyNumbers: [{ text: `$${n}M`, label: 'bookings' }],
  ...o,
});
const stored = (c: string, o: Partial<AgentThreadTurn> = {}): AgentThreadTurn => ({
  turnUuid: uuidOf(T(c)),
  question: `Stored question ${c}?`,
  askedAt: at(10, 42),
  status: 'answered',
  summaryMd: 'Finance is **higher**.',
  parts: [part(1), part(2)],
  ...o,
});
const thread = (id: string, turns: AgentThreadTurn[]): AgentThread => ({
  threadId: id,
  mogId: MOG,
  mogName: 'Sales MOG',
  turns,
});
const genieRow = (n: number, over: Partial<ConversationRow> = {}): ConversationRow => ({
  requestId: `41111111-2222-4333-8444-${String(n).padStart(12, '0')}`,
  mogId: MOG,
  mogName: 'Sales MOG',
  question: `Genie question ${n}?`,
  answerMd: 'A fake answer.',
  keyNumbers: [],
  askedAt: at(9, n),
  profile: 'fake-west',
  workspaceHost: 'example.cloud.databricks.test',
  status: 'answered',
  conversationId: `conv-${n}`,
  ...over,
});

const READY = {
  binary: 'ready',
  pinned: '0.5.0',
  version: '0.5.0',
  agentProfile: null,
  agentProfileState: 'ready',
} as const;
const SESSION: HubAgentView['session'] = {
  state: 'busy',
  sessionId: T('d'),
  startedAt: at(10, 42),
  agentProfile: 'fake-sales',
  turns: 3,
  queued: 2,
  tokens: 124_000,
};
const view = (o: Partial<HubAgentView> = {}): HubAgentView => ({
  session: { state: 'none', agentProfile: null, turns: 0, queued: 0, tokens: 0 },
  waiting: [],
  earlier: [],
  genieCode: READY,
  threads: [],
  ...o,
});
const running = view({
  session: SESSION,
  live: memTurn('a', { state: 'running', used: 1, budget: 4 }),
  waiting: [
    memTurn('b', { state: 'queued', position: 1, calls: [] }),
    memTurn('c', { state: 'queued', position: 2, calls: [] }),
  ],
  threads: [
    { turnId: T('a'), threadId: THREAD_A },
    { turnId: T('b'), threadId: THREAD_A },
    { turnId: T('c'), threadId: THREAD_A },
  ],
});

const q = (el: HTMLElement, sel: string) => el.querySelector<HTMLElement>(sel);
const options = (el: HTMLElement) => [...el.querySelectorAll('[role="option"]')] as HTMLElement[];
const ids = (el: HTMLElement) => options(el).map((o) => o.getAttribute('data-request'));
const click = (el: Element | null | undefined) => act(async () => (el as HTMLElement).click());
const key = (target: HTMLElement, k: string) =>
  act(async () => {
    target.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));
  });
const payloads = (scope?: string) =>
  viewEmits()
    .filter((e) => scope === undefined || (e as { mogId?: string }).mogId === scope)
    .map((e) => (e as { payload?: Record<string, unknown> }).payload ?? {});
const deletes = () =>
  payloads(CONVERSATIONS_VIEW_ID).filter((p) => p['action'] === 'conversations.delete');
const type = (el: HTMLTextAreaElement, text: string) => {
  const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  set?.call(el, text);
  el.dispatchEvent(new Event('input', { bubbles: true }));
};

function Host({ active = true }: { active?: boolean }) {
  const agent = useHubAgent();
  return (
    <Conversations
      active={active}
      onOpen={vi.fn()}
      now={() => NOW}
      agent={agent}
      agentMogId={MOG}
    />
  );
}

const mounted: Root[] = [];
async function mount(active = true) {
  const r = await render(<Host active={active} />);
  mounted.push(r.root);
  return r;
}
afterEach(async () => {
  for (const r of mounted.splice(0)) await act(async () => r.unmount());
  vi.useRealTimers();
});
const feed = async (
  agentThreads: AgentThread[],
  rows: ConversationRow[] = [],
  v?: HubAgentView,
) => {
  await tauri.emitTo('hub', HUB_CONVERSATIONS_EVENT, { rows, agentThreads });
  if (v) await tauri.emitTo('hub', HUB_AGENT_EVENT, v);
};

describe('agent threads in the list', () => {
  beforeEach(() => tauri.reset());

  it('lists a stored agent thread with its first question, MOG, time and workspaces', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')])]);
    const row = options(el)[0] as HTMLElement;
    expect(row.getAttribute('data-request')).toBe(`a:${THREAD_A}`);
    expect(row.textContent).toContain('Stored question a?');
    expect(row.textContent).toContain('Sales MOG');
    expect(row.textContent).toMatch(/10:42|10:42/);
    expect(row.textContent).toContain('Sales, Finance');
    expect(row.textContent).not.toContain('Failed');
  });

  it('shows "Failed" when no turn answered and "Stopped" when the last turn stopped', async () => {
    const { el } = await mount();
    await feed([
      thread(THREAD_A, [stored('a', { status: 'failed', summaryMd: '', parts: [] })]),
      thread(THREAD_B, [stored('b'), stored('c', { status: 'stopped', summaryMd: '', parts: [] })]),
    ]);
    const text = (id: string) => el.querySelector(`[data-request="a:${id}"]`)?.textContent;
    expect(text(THREAD_A)).toContain('Failed');
    expect(text(THREAD_B)).toContain('Stopped');
  });

  it('[AC-C8] a live turn shows in the In progress group, above the days, with its state', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_B, [stored('e')])], [genieRow(1)], running);
    const group = el.querySelector('[role="group"][aria-label="In progress"]') as HTMLElement;
    expect(group).not.toBeNull();
    expect(group.textContent).toContain('Compare a?');
    expect(group.textContent).toContain('Asking · 1 of 4 Genie calls');
    expect(group.querySelector('[data-live-dot]')).not.toBeNull();
    expect(group.querySelector('[data-live-dot]')?.getAttribute('aria-hidden')).toBe('true');
    expect(ids(el)[0]).toBe(`a:${THREAD_A}`);
    // The days come after it.
    const groups = [...el.querySelectorAll('[role="group"]')].map((g) =>
      g.getAttribute('aria-label'),
    );
    expect(groups[0]).toBe('In progress');
    expect(groups).toContain('Today');
  });

  it('a thread that only waits shows a Queued pill with the copy', async () => {
    const { el } = await mount();
    const v = view({
      session: SESSION,
      waiting: [memTurn('b', { state: 'queued', position: 1, calls: [] })],
      threads: [{ turnId: T('b'), threadId: THREAD_A }],
    });
    await feed([], [], v);
    const row = el.querySelector('[data-in-progress]') as HTMLElement;
    expect(row.textContent).toContain('Queued');
    expect(row.textContent).toContain('Sends when the answer above lands');
    expect(row.querySelector('[data-live-dot]')).toBeNull();
  });

  it('a slow turn says "Still working…" and a planning turn says "Planning…"', async () => {
    const { el } = await mount();
    const slow = { ...running, live: memTurn('a', { state: 'slow' }) };
    await feed([], [], slow);
    expect(el.querySelector('[data-in-progress]')?.textContent).toContain('Still working…');
    await tauri.emitTo('hub', HUB_AGENT_EVENT, {
      ...running,
      live: memTurn('a', { state: 'planning', calls: [] }),
    });
    expect(el.querySelector('[data-in-progress]')?.textContent).toContain('Planning…');
  });

  it('the in-memory turn wins over the stored turn with the same turn id', async () => {
    const { el } = await mount();
    const v = view({
      session: SESSION,
      earlier: [memTurn('a', { question: 'Live copy of the question?' })],
      threads: [{ turnId: T('a'), threadId: THREAD_A }],
    });
    await feed([thread(THREAD_A, [stored('a')])], [], v);
    const detail = el.querySelector('[data-conversation-detail]') as HTMLElement;
    expect(detail.textContent).toContain('Live copy of the question?');
    expect(detail.textContent).not.toContain('Stored question a?');
    expect(detail.querySelectorAll('h2')).toHaveLength(1);
  });

  it('the thread opens by its id, also before the store has it', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')]), thread(THREAD_B, [stored('b')])]);
    await tauri.emitTo('hub', HUB_CONVERSATIONS_OPEN_EVENT, { threadId: THREAD_B });
    expect(el.querySelector('[aria-selected="true"]')?.getAttribute('data-request')).toBe(
      `a:${THREAD_B}`,
    );
    // Not in the store yet: the selection waits and takes it once it comes.
    await tauri.emitTo('hub', HUB_CONVERSATIONS_OPEN_EVENT, { threadId: THREAD_C });
    await feed([thread(THREAD_A, [stored('a')]), thread(THREAD_C, [stored('c')])]);
    expect(el.querySelector('[aria-selected="true"]')?.getAttribute('data-request')).toBe(
      `a:${THREAD_C}`,
    );
  });
});

describe('the agent thread detail', () => {
  beforeEach(() => tauri.reset());

  it('a stored turn shows the question, the note and the parts behind one toggle', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')])]);
    const d = el.querySelector('[data-conversation-detail]') as HTMLElement;
    expect(d.querySelector('h2')?.textContent).toBe('Stored question a?');
    expect(d.querySelector('[data-agent-note]')?.textContent).toContain('Note from your MOG');
    expect(d.querySelector('[data-agent-note]')?.textContent).toContain('Finance is higher.');
    const toggle = d.querySelector<HTMLElement>('[data-action="parts"]') as HTMLElement;
    expect(toggle.textContent).toBe('From each workspace');
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    const list = d.querySelector<HTMLElement>(
      `#${toggle.getAttribute('aria-controls')}`,
    ) as HTMLElement;
    expect(list.hidden).toBe(true);
    await click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(list.hidden).toBe(false);
    expect(list.textContent).toContain('Answer 1 from a workspace.');
    expect(list.querySelectorAll('[data-agent-chip]').length).toBeGreaterThanOrEqual(2);
    expect(list.textContent).toContain('$1M bookings');
  });

  it('a turn with no note shows its parts open, with no toggle', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a', { summaryMd: '' })])]);
    const d = el.querySelector('[data-conversation-detail]') as HTMLElement;
    expect(d.querySelector('[data-agent-note]')).toBeNull();
    expect(d.querySelector('[data-action="parts"]')).toBeNull();
    expect((d.querySelector('.agent-parts') as HTMLElement).hidden).toBe(false);
  });

  it('turns show oldest first, one heading each, and a failed part says so', async () => {
    const { el } = await mount();
    await feed([
      thread(THREAD_A, [
        stored('a', { question: 'First?' }),
        stored('b', {
          question: 'Second?',
          status: 'failed',
          summaryMd: '',
          parts: [part(1, { status: 'failed', answerMd: '', keyNumbers: [] })],
        }),
      ]),
    ]);
    const d = el.querySelector('[data-conversation-detail]') as HTMLElement;
    expect([...d.querySelectorAll('h2')].map((h) => h.textContent)).toEqual(['First?', 'Second?']);
    expect(d.textContent).toContain('Genie could not answer this question.');
    expect(d.getAttribute('aria-labelledby')).toBe(d.querySelector('h2')?.id);
  });

  it('an ended session says so, with no New session button', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')])], [], view());
    const d = el.querySelector('[data-conversation-detail]') as HTMLElement;
    expect(d.querySelector('[data-agent-ended]')?.textContent).toBe(
      'This session ended. A reply starts a new session.',
    );
    expect(d.querySelector('[data-agent-new-session]')).toBeNull();
  });

  it('a thread of the live session shows the session line and New session', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')])], [], {
      ...view({ session: SESSION }),
      waiting: [],
      earlier: [memTurn('e')],
      threads: [{ turnId: T('e'), threadId: THREAD_A }],
    });
    const line = q(el, '[data-agent-session]');
    expect(line?.textContent).toMatch(
      /^Genie Code in Sales · started .+ · 3 questions · 124k tokens$/,
    );
    // No turn runs: New session sends its action at once and the focus goes to the line.
    await click(q(el, '[data-agent-new-session]'));
    expect(payloads(HUB_SCOPE)).toContainEqual({ action: 'agent.new_session' });
  });

  it('[AC-C10] New session asks first while a turn is live, and Cancel has the focus', async () => {
    const { el } = await mount();
    await feed([], [], running);
    await click(q(el, '[data-agent-new-session]'));
    const confirm = q(el, '[data-agent-new-session-confirm]') as HTMLElement;
    expect(confirm.textContent).toContain('The live question stops');
    expect(document.activeElement?.textContent).toBe('Cancel');
    expect(payloads(HUB_SCOPE)).not.toContainEqual({ action: 'agent.new_session' });
    await click(q(el, '[data-confirm-new-session]'));
    expect(payloads(HUB_SCOPE)).toContainEqual({ action: 'agent.new_session' });
  });

  it('the live turn shows its calls, "n of N" and a Stop button with a full name', async () => {
    const { el } = await mount();
    await feed([], [], running);
    const live = q(el, '[data-agent-live]') as HTMLElement;
    expect(live.querySelector('h2')?.textContent).toBe('Compare a?');
    expect(live.querySelectorAll('[data-agent-call]')).toHaveLength(2);
    expect(live.textContent).toContain('1 of 4 Genie calls');
    const stop = q(el, '[data-agent-stop]') as HTMLElement;
    expect(stop.getAttribute('aria-label')).toBe('Stop: Compare a?');
    await click(stop);
    expect(payloads(HUB_SCOPE)).toContainEqual({ action: 'agent.stop', turnId: T('a') });
    // Only the live turn is a live region.
    expect(el.querySelectorAll('[aria-live="polite"][data-agent-headline]')).toHaveLength(1);
  });

  it('a slow turn shows "Still working…" in the detail', async () => {
    const { el } = await mount();
    await feed([], [], { ...running, live: memTurn('a', { state: 'slow' }) });
    expect(q(el, '[data-agent-live]')?.textContent).toContain('Still working…');
  });

  it('[AC-C9] waiting rows show the copy and a × that removes only that row', async () => {
    const { el } = await mount();
    await feed([], [], running);
    const rows = [...el.querySelectorAll('[data-agent-waiting]')];
    expect(rows).toHaveLength(2);
    expect(rows[0]?.textContent).toContain('Sends when the answer above lands');
    const x = rows[0]?.querySelector('button') as HTMLElement;
    expect(x.getAttribute('aria-label')).toBe('Remove: Compare b?');
    await click(x);
    expect(payloads(HUB_SCOPE)).toContainEqual({ action: 'agent.remove_queued', turnId: T('b') });
    // The focus goes to the next row's ×.
    expect(document.activeElement).toBe(rows[1]?.querySelector('button'));
  });

  it('a needs_you turn shows Open Settings', async () => {
    const { el } = await mount();
    const v = view({
      session: SESSION,
      earlier: [memTurn('e', { state: 'needs_you', notice: 'Sign in to Sales again.' })],
      threads: [{ turnId: T('e'), threadId: THREAD_A }],
    });
    await feed([], [], v);
    expect(q(el, '[data-agent-earlier]')?.textContent).toContain('Sign in to Sales again.');
    await click(q(el, '[data-agent-open-settings]'));
    expect(payloads(HUB_SCOPE)).toContainEqual({ action: 'agent.open_settings' });
  });
});

describe('the composer in an agent thread', () => {
  beforeEach(() => tauri.reset());

  it('is "Ask a follow-up" and sends agent.thread_send for this thread and MOG', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')])], [], view());
    const field = el.querySelector('textarea') as HTMLTextAreaElement;
    expect(field.placeholder).toBe('Ask a follow-up');
    await act(async () => type(field, 'And next quarter?'));
    await click(el.querySelector('button[type="submit"]'));
    expect(payloads(CONVERSATIONS_VIEW_ID)).toContainEqual({
      action: 'agent.thread_send',
      threadId: THREAD_A,
      mogId: MOG,
      question: 'And next quarter?',
    });
    expect(field.value).toBe('');
  });

  it('sends nothing for a blank question', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')])], [], view());
    expect((el.querySelector('button[type="submit"]') as HTMLButtonElement).disabled).toBe(true);
  });

  it('announces thread.active_agent while it shows, and thread.inactive when it goes', async () => {
    const { el, root } = await mount();
    await feed([thread(THREAD_A, [stored('a')])], [], view());
    expect(payloads(CONVERSATIONS_VIEW_ID)).toContainEqual({
      action: 'thread.active_agent',
      threadId: THREAD_A,
    });
    expect(el.querySelector('[data-agent-composer]')).not.toBeNull();
    await act(async () => root.render(<Host active={false} />));
    expect(payloads(CONVERSATIONS_VIEW_ID).at(-1)).toEqual({ action: 'thread.inactive' });
  });

  it('with Genie Code not ready it says why and offers Open Settings', async () => {
    const { el } = await mount();
    const off = view({
      genieCode: { ...READY, binary: 'not_installed' },
    });
    await feed([thread(THREAD_A, [stored('a')])], [], off);
    expect(el.querySelector('textarea')).toBeNull();
    expect(q(el, '[data-agent-composer="not-ready"]')?.textContent).toContain(
      'Genie Code is not installed.',
    );
    await click(q(el, '[data-agent-open-settings]'));
    expect(payloads(HUB_SCOPE)).toContainEqual({ action: 'agent.open_settings' });
  });

  it('with three questions waiting it keeps the text and turns Send off', async () => {
    const { el } = await mount();
    const three = view({
      session: SESSION,
      live: memTurn('a', { state: 'running' }),
      waiting: ['b', 'c', 'd'].map((c) => memTurn(c, { state: 'queued', calls: [] })),
      threads: ['a', 'b', 'c', 'd'].map((c) => ({ turnId: T(c), threadId: THREAD_A })),
    });
    await feed([], [], three);
    expect(q(el, '[data-agent-queue-full]')?.textContent).toBe(
      '3 questions are waiting. Send this when one lands.',
    );
    const field = el.querySelector('textarea') as HTMLTextAreaElement;
    await act(async () => type(field, 'One more?'));
    expect((el.querySelector('button[type="submit"]') as HTMLButtonElement).disabled).toBe(true);
    expect(field.value).toBe('One more?');
  });

  it('while a turn runs, Send queues (it stays on)', async () => {
    const { el } = await mount();
    await feed([], [], running);
    const field = el.querySelector('textarea') as HTMLTextAreaElement;
    await act(async () => type(field, 'Then this?'));
    expect((el.querySelector('button[type="submit"]') as HTMLButtonElement).disabled).toBe(false);
  });
});

describe('Delete, Clear failed and Undo', () => {
  beforeEach(() => {
    tauri.reset();
    vi.useFakeTimers();
  });

  const two = () => [
    thread(THREAD_A, [stored('a')]),
    thread(THREAD_B, [
      stored('b', { status: 'failed', summaryMd: '', parts: [], askedAt: at(9, 0) }),
    ]),
  ];

  it('[AC-C11] Delete hides the thread at once and shows Undo for 8 s', async () => {
    const { el } = await mount();
    await feed(two());
    await click(q(el, '[data-action="delete"]'));
    expect(ids(el)).toEqual([`a:${THREAD_B}`]);
    expect(q(el, '[data-conversations-undo]')?.textContent).toContain('Conversation deleted');
    expect(q(el, '[data-conversations-undo] [data-action="undo"]')).not.toBeNull();
    // The selection moved to the next row, and the focus is in the list.
    expect(el.querySelector('[aria-selected="true"]')?.getAttribute('data-request')).toBe(
      `a:${THREAD_B}`,
    );
    expect(document.activeElement).toBe(el.querySelector('[role="listbox"]'));
    await act(async () => void vi.advanceTimersByTime(MEMORY_UNDO_MS - 1));
    expect(q(el, '[data-conversations-undo]')).not.toBeNull();
    expect(deletes()).toHaveLength(0);
  });

  it('[AC-C12] the view sends the delete after 8 s with no Undo, and the toast goes', async () => {
    const { el } = await mount();
    await feed(two());
    await click(q(el, '[data-action="delete"]'));
    await act(async () => void vi.advanceTimersByTime(MEMORY_UNDO_MS));
    expect(deletes()).toEqual([{ action: 'conversations.delete', threads: [`a:${THREAD_A}`] }]);
    expect(q(el, '[data-conversations-undo]')).toBeNull();
  });

  it('[AC-C13] Undo brings the thread back and sends nothing', async () => {
    const { el } = await mount();
    await feed(two());
    await click(q(el, '[data-action="delete"]'));
    await click(q(el, '[data-conversations-undo] [data-action="undo"]'));
    expect(ids(el)).toEqual([`a:${THREAD_A}`, `a:${THREAD_B}`]);
    expect(el.querySelector('[aria-selected="true"]')?.getAttribute('data-request')).toBe(
      `a:${THREAD_A}`,
    );
    await act(async () => void vi.advanceTimersByTime(MEMORY_UNDO_MS * 2));
    expect(deletes()).toHaveLength(0);
  });

  it('a list refresh in the 8 s keeps the pending thread hidden', async () => {
    const { el } = await mount();
    await feed(two());
    await click(q(el, '[data-action="delete"]'));
    await feed(two());
    expect(ids(el)).toEqual([`a:${THREAD_B}`]);
    await act(async () => void vi.advanceTimersByTime(MEMORY_UNDO_MS));
    // Sent, and still hidden until the next list shows the truth; a failed delete shows again.
    expect(ids(el)).toEqual([`a:${THREAD_B}`]);
    await feed(two());
    expect(ids(el)).toEqual([`a:${THREAD_A}`, `a:${THREAD_B}`]);
  });

  it('[pagehide] a pending delete goes at once when the window closes', async () => {
    const { el } = await mount();
    await feed(two());
    await click(q(el, '[data-action="delete"]'));
    await act(async () => void window.dispatchEvent(new Event('pagehide')));
    expect(deletes()).toEqual([{ action: 'conversations.delete', threads: [`a:${THREAD_A}`] }]);
    await act(async () => void vi.advanceTimersByTime(MEMORY_UNDO_MS * 2));
    expect(deletes()).toHaveLength(1);
  });

  it('a pending delete goes at once when the area unmounts', async () => {
    const { el, root } = await mount();
    await feed(two());
    await click(q(el, '[data-action="delete"]'));
    await act(async () => root.unmount());
    mounted.splice(0);
    expect(deletes()).toHaveLength(1);
  });

  it('a new delete sends the pending one at once and keeps one toast', async () => {
    const { el } = await mount();
    await feed([...two(), thread(THREAD_C, [stored('c', { askedAt: at(8, 0) })])]);
    await click(q(el, '[data-action="delete"]'));
    await click(q(el, '[data-action="delete"]'));
    expect(deletes()).toEqual([{ action: 'conversations.delete', threads: [`a:${THREAD_A}`] }]);
    expect(el.querySelectorAll('[data-conversations-undo]')).toHaveLength(1);
    await act(async () => void vi.advanceTimersByTime(MEMORY_UNDO_MS));
    expect(deletes()).toHaveLength(2);
    expect(deletes()[1]?.['threads']).toEqual([`a:${THREAD_B}`]);
  });

  it('Delete and Backspace on the list do the same as the button', async () => {
    const { el } = await mount();
    await feed(two());
    await key(el.querySelector('[role="listbox"]') as HTMLElement, 'Delete');
    expect(ids(el)).toEqual([`a:${THREAD_B}`]);
    await key(el.querySelector('[role="listbox"]') as HTMLElement, 'Backspace');
    expect(el.textContent).toContain('No conversations yet.');
    // One toast only: the second delete sent the first.
    expect(deletes()).toHaveLength(1);
  });

  it('a Genie thread deletes with its conversation key, and a blank one with its request key', async () => {
    const { el } = await mount();
    await feed([], [genieRow(2), genieRow(1, { conversationId: '' })]);
    await click(q(el, '[data-action="delete"]'));
    await click(q(el, '[data-action="delete"]'));
    await act(async () => void vi.advanceTimersByTime(MEMORY_UNDO_MS));
    expect(deletes().map((d) => d['threads'])).toEqual([
      ['c:conv-2'],
      [`r:${genieRow(1).requestId}`],
    ]);
  });

  it('[AC-C14] Clear failed counts only threads with no answered exchange, and Undo restores them', async () => {
    const { el } = await mount();
    const rows = [
      genieRow(5, { status: 'failed', answerMd: '' }),
      genieRow(4, { conversationId: 'conv-x', status: 'failed', answerMd: '' }),
      genieRow(3, { conversationId: 'conv-x' }),
    ];
    await feed(
      [
        ...two(),
        thread(THREAD_C, [
          stored('c', { status: 'stopped', summaryMd: '', parts: [], askedAt: at(8, 0) }),
        ]),
      ],
      rows,
    );
    const btn = q(el, '[data-action="clear-failed"]') as HTMLElement;
    // genie 5 (failed), agent B (failed), agent C (stopped): conv-x has an answer.
    expect(btn.textContent).toBe('Clear failed (3)');
    await click(btn);
    expect(q(el, '[data-conversations-undo]')?.textContent).toContain(
      '3 failed conversations cleared',
    );
    expect(q(el, '[data-action="clear-failed"]')).toBeNull();
    expect(options(el)).toHaveLength(2);
    await click(q(el, '[data-conversations-undo] [data-action="undo"]'));
    expect(options(el)).toHaveLength(5);
    expect(deletes()).toHaveLength(0);
    // Again, and let it go: one batch.
    await click(q(el, '[data-action="clear-failed"]'));
    await act(async () => void vi.advanceTimersByTime(MEMORY_UNDO_MS));
    expect(deletes()).toHaveLength(1);
    expect([...(deletes()[0]?.['threads'] as string[])].sort()).toEqual(
      ['c:conv-5', `a:${THREAD_B}`, `a:${THREAD_C}`].sort(),
    );
  });

  it('[AC-C15] Clear failed never counts a thread in progress, and an in-progress thread has no Delete', async () => {
    const { el } = await mount();
    await feed(
      [
        thread(THREAD_A, [stored('f', { status: 'failed', summaryMd: '', parts: [] })]),
        thread(THREAD_B, [stored('g', { status: 'failed', summaryMd: '', parts: [] })]),
      ],
      [],
      running,
    );
    expect(q(el, '[data-action="clear-failed"]')?.textContent).toBe('Clear failed (1)');
    expect(options(el)[0]?.getAttribute('data-request')).toBe(`a:${THREAD_A}`);
    // The thread that runs is the selected one: no Delete button, and the key does nothing.
    expect(q(el, '[data-action="delete"]')).toBeNull();
    await key(el.querySelector('[role="listbox"]') as HTMLElement, 'Delete');
    expect(q(el, '[data-conversations-undo]')).toBeNull();
    expect(options(el)).toHaveLength(2);
  });

  it('with one failed thread the toast is singular, and the button shows only for n of 1 or more', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_B, [stored('b', { status: 'failed', summaryMd: '', parts: [] })])]);
    await click(q(el, '[data-action="clear-failed"]'));
    expect(q(el, '[data-conversations-undo]')?.textContent).toContain(
      '1 failed conversation cleared',
    );
    expect(el.textContent).toContain('No conversations yet.');
    await click(q(el, '[data-conversations-undo] [data-action="undo"]'));
    expect(options(el)).toHaveLength(1);
  });

  it('the Delete button has a hint, and the toast is a polite status region', async () => {
    const { el } = await mount();
    await feed([], [genieRow(1)]);
    expect(q(el, '[data-action="delete"]')?.getAttribute('title')).toBe(
      'Deletes it from this Mac. The Genie conversation stays in Genie.',
    );
    expect(q(el, '.conv-toast-live')?.getAttribute('role')).toBe('status');
  });
});

describe('the hub has no Agent area (AC-C6)', () => {
  beforeEach(() => tauri.reset());

  it('[AC-C6] no Agent entry in the sidebar, no Agent section, and a hub:agent notice moves nothing', async () => {
    const { el } = await render(<Hub />);
    await act(async () => el.querySelector<HTMLButtonElement>('[data-sidebar-toggle]')?.click());
    const items = [...el.querySelectorAll('[data-sidebar-item]')].map((n) =>
      n.getAttribute('data-sidebar-item'),
    );
    expect(items).not.toContain('agent');
    expect(el.textContent).not.toMatch(/\bAgent\b/);
    await tauri.emitTo('hub', HUB_AGENT_EVENT, view({ notice: { kind: 'info', text: 'Hello.' } }));
    expect(el.querySelector('[data-hub-view="agent"]')).toBeNull();
    expect(el.querySelector('.hub-area-title')?.textContent).toBe('Home');
  });

  it('the hub listens to hub:agent for this window only and sends agent.ready once', async () => {
    await render(<Hub />);
    expect(tauri.listenOptions.get(HUB_AGENT_EVENT)).toEqual([{ target: 'hub' }]);
    expect(payloads(HUB_SCOPE).filter((p) => p['action'] === 'agent.ready')).toEqual([
      { action: 'agent.ready' },
    ]);
  });
});
