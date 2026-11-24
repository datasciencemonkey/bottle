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
import { HUB_ORCHESTRATOR_EVENT } from '../../shared/hub-orchestrator';
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

/** The brain sends the orchestrator view (ADR 0122): the composer reads its state. */
const orchestratorState = (state: string) =>
  tauri.emitTo('hub', HUB_ORCHESTRATOR_EVENT, {
    models: [{ id: 'system.ai.claude-haiku-5-5', label: 'Claude Haiku 5.5' }],
    selected: 'system.ai.claude-haiku-5-5',
    cleanDictation: true,
    state,
    homeName: 'Sales',
  });

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
    expect(row.textContent).toContain('Sends next');
    // One line in the list: no wrap onto a second one.
    expect(row.querySelector('.conv-row-meta')?.textContent).not.toContain('answer above');
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

  it('[owner bug] a stored turn shows the answers open and first, the note after them as a muted block', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')])]);
    const d = el.querySelector('[data-conversation-detail]') as HTMLElement;
    expect(d.querySelector('h2')?.textContent).toBe('Stored question a?');
    const titles = [...d.querySelectorAll('h3.agent-part-title')].map((h) => h.textContent);
    expect(titles).toEqual(['Answer from Sales', 'Answer from Finance']);
    const list = d.querySelector<HTMLElement>('.agent-parts') as HTMLElement;
    expect(list.hidden).toBe(false);
    expect(list.textContent).toContain('Answer 1 from a workspace.');
    expect(list.textContent).toContain('$1M bookings');
    const note = d.querySelector('[data-agent-note]') as HTMLElement;
    expect(note.className).toContain('agent-note--after');
    expect(note.textContent).toContain('Note from your MOG');
    expect(note.textContent).toContain('Finance is higher.');
    expect(list.compareDocumentPosition(note) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('[P2-8] the answers have a toggle with a chevron and a label that says what it does', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')])]);
    const d = el.querySelector('[data-conversation-detail]') as HTMLElement;
    const toggle = d.querySelector<HTMLElement>('[data-action="parts"]') as HTMLElement;
    expect(toggle.textContent).toBe('Hide workspace answers');
    expect(toggle.querySelector('svg')).not.toBeNull();
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    const list = d.querySelector<HTMLElement>(`#${toggle.getAttribute('aria-controls')}`);
    await click(toggle);
    expect(toggle.textContent).toBe('2 workspace answers');
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(list?.hidden).toBe(true);
  });

  it('[P2-6] a stored part has its workspace colour stripe', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')])]);
    const parts = [...el.querySelectorAll<HTMLElement>('[data-agent-part]')];
    expect(parts).toHaveLength(2);
    for (const p of parts) expect(p.style.getPropertyValue('--agent-ws-color')).toMatch(/^#|rgb/);
    expect(parts[0]?.style.getPropertyValue('--agent-ws-color')).not.toBe(
      parts[1]?.style.getPropertyValue('--agent-ws-color'),
    );
  });

  it('[P2-7] two parts of one workspace show one chip and no key clash', async () => {
    const { el } = await mount();
    const errors = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    await feed([
      thread(THREAD_A, [
        stored('a', {
          parts: [
            part(1),
            part(2, { requestId: '62222222-3333-4444-8555-0000000000ff', workspace: { ...sales } }),
          ],
        }),
      ]),
    ]);
    const meta = el.querySelector('.conv-exchange .conv-chips') as HTMLElement;
    expect(meta.querySelectorAll('[data-agent-chip]')).toHaveLength(1);
    expect(errors).not.toHaveBeenCalled();
    errors.mockRestore();
  });

  it('[P3-12] a failed turn with one failed part shows the failure once', async () => {
    const { el } = await mount();
    await feed([
      thread(THREAD_A, [
        stored('a', {
          status: 'failed',
          summaryMd: '',
          parts: [part(2, { status: 'failed', answerMd: '', keyNumbers: [] })],
        }),
      ]),
    ]);
    const d = el.querySelector('[data-conversation-detail]') as HTMLElement;
    expect(d.querySelectorAll('.agent-tag')).toHaveLength(0);
    expect(d.querySelector('.conv-meta .conv-chips')).toBeNull();
    expect(d.textContent).toContain('Genie could not answer this question.');
  });

  it('[P3-13] a stopped turn with no parts and no note says what happened', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a', { status: 'stopped', summaryMd: '', parts: [] })])]);
    expect(el.querySelector('[data-conversation-detail]')?.textContent).toContain(
      'You stopped this question before an answer came.',
    );
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

  it('[M4] a session open for another thread does not say "This session ended"', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')]), thread(THREAD_B, [stored('b')])], [], {
      ...view({ session: SESSION }),
      earlier: [memTurn('e')],
      threads: [{ turnId: T('e'), threadId: THREAD_B }],
    });
    // THREAD_A is selected (newest first): it is not in the open session.
    const d = el.querySelector('[data-conversation-detail]') as HTMLElement;
    expect(d.getAttribute('data-agent-thread')).toBe(THREAD_A);
    expect(d.querySelector('[data-agent-ended]')).toBeNull();
    expect(d.textContent).not.toContain('This session ended');
    expect(d.querySelector('[data-agent-new-session]')).toBeNull();
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
      /^MOG agent in Sales · started .+ · 3 questions · 124k tokens$/,
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

  it('with the orchestrator model not ready it says why and offers Open Settings', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')])], [], view());
    await orchestratorState('unreachable');
    expect(el.querySelector('textarea')).toBeNull();
    expect(q(el, '[data-agent-composer="not-ready"]')?.textContent).toContain(
      'The orchestrator model is not reachable. Questions go to your home workspace only.',
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
    // Only Clear failed carries the flag; a plain Delete (tests above) sends none.
    expect(deletes()[0]?.['onlyFailed']).toBe(true);
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
      "Deletes it from this Mac. The Genie conversation stays in Genie until you Wipe or Forget with 'Also delete Genie conversations'.",
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

describe('review fixes (needs you, queued only, toast, composer, list line)', () => {
  beforeEach(() => tauri.reset());

  it('[P1-1] a thread that needs the user shows "Needs you", not "Failed", and Clear failed keeps it', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')])], [], {
      ...view({ session: SESSION }),
      earlier: [memTurn('e', { state: 'needs_you', calls: [], parts: [] })],
      threads: [{ turnId: T('e'), threadId: THREAD_B }],
    });
    const row = options(el).find((o) => o.getAttribute('data-request') === `a:${THREAD_B}`);
    expect(row?.textContent).toContain('Needs you');
    expect(row?.textContent).not.toContain('Failed');
    expect(row?.querySelector('.conv-status--needs')).not.toBeNull();
    expect(q(el, '[data-action="clear-failed"]')).toBeNull();
  });

  it('[P1-4] a thread with only a queued turn has a heading, an honest line and a way to remove it', async () => {
    const { el } = await mount();
    const v = view({
      session: SESSION,
      waiting: [memTurn('b', { state: 'queued', position: 1, calls: [] })],
      threads: [{ turnId: T('b'), threadId: THREAD_A }],
    });
    await feed([], [], v);
    const d = el.querySelector('[data-conversation-detail]') as HTMLElement;
    const h = d.querySelector('h2') as HTMLElement;
    expect(h.textContent).toBe('Compare b?');
    expect(d.getAttribute('aria-labelledby')).toBe(h.id);
    expect(d.textContent).toContain('Sends after the current question');
    expect(d.textContent).not.toContain('answer above');
    await click(d.querySelector(`[data-agent-remove="${T('b')}"]`));
    expect(payloads(HUB_SCOPE)).toContainEqual({ action: 'agent.remove_queued', turnId: T('b') });
  });

  it('[P1-4] a queued turn under an answer keeps "Sends when the answer above lands"', async () => {
    const { el } = await mount();
    await feed([], [], running);
    expect(el.querySelector('[data-agent-waiting]')?.textContent).toContain(
      'Sends when the answer above lands',
    );
  });

  it('[P1-3] the Undo toast sits in the list column, never over the composer', async () => {
    const { el } = await mount();
    await feed([], [genieRow(1), genieRow(2)]);
    await click(q(el, '[data-action="delete"]'));
    const toast = q(el, '[data-conversations-undo]') as HTMLElement;
    expect(toast.closest('.conv-listcol')).not.toBeNull();
    expect(toast.closest('[data-conversation-detail]')).toBeNull();
  });

  it('[P2-9] Delete sits in the session row of an agent thread, with no bar of its own', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')])], [], view());
    const d = el.querySelector('[data-conversation-detail]') as HTMLElement;
    expect(d.querySelector('.conv-detail-bar')).toBeNull();
    expect(d.querySelector('.agent-session-row [data-action="delete"]')).not.toBeNull();
  });

  it('[P2-10] after Clear failed empties the list the focus goes to the empty state', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a', { status: 'failed', summaryMd: '', parts: [] })])]);
    await click(q(el, '[data-action="clear-failed"]'));
    const empty = q(el, '.conv-message') as HTMLElement;
    expect(empty.getAttribute('tabindex')).toBe('-1');
    expect(document.activeElement).toBe(empty);
    // The toast stays reachable.
    expect(q(el, '[data-conversations-undo]')).not.toBeNull();
  });

  it('[P3-16] Clear failed is a quiet secondary action', async () => {
    const { el } = await mount();
    await feed([], [genieRow(1, { status: 'failed', answerMd: '' })]);
    expect(q(el, '[data-action="clear-failed"]')?.className).toContain('text-muted-foreground');
  });

  it('[P3-14] the agent composer and the not-ready Settings button match the Genie composer', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')])], [], view());
    await orchestratorState('no_model');
    expect(q(el, '[data-agent-open-settings]')?.className).toContain('border');
    await feed([], [], { ...view(), earlier: [memTurn('e', { state: 'needs_you' })] });
  });

  it('[LOW] the composer stops at 1000 characters and says so; nothing is lost silently', async () => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a')])], [], view());
    const field = el.querySelector('textarea') as HTMLTextAreaElement;
    expect(field.maxLength).toBe(1000);
    expect(q(el, '[data-agent-too-long]')).toBeNull();
    await act(async () => type(field, 'x'.repeat(1000)));
    expect(q(el, '[data-agent-too-long]')?.textContent).toBe(
      'A question can have up to 1000 characters.',
    );
    await click(el.querySelector('button[type="submit"]'));
    const sent = payloads(CONVERSATIONS_VIEW_ID).find((p) => p['action'] === 'agent.thread_send');
    expect((sent?.['question'] as string).length).toBe(1000);
  });

  it('[owner bug] a finished agent thread lists its first answer line as the row hint, never the note', async () => {
    const { el } = await mount();
    await feed([
      thread(THREAD_A, [
        stored('a', {
          summaryMd: 'Its findings were shown directly to you.',
          parts: [
            part(1, { status: 'failed', answerMd: '' }),
            part(2, { answerMd: '\n## Churn fell 4%\nMore.' }),
          ],
        }),
      ]),
    ]);
    const row = options(el)[0] as HTMLElement;
    expect(row.getAttribute('title')).toBe('Churn fell 4%');
  });

  it('[owner bug] a turn of this session shows its workspace answers open and first in the detail', async () => {
    const { el } = await mount();
    await feed([], [], {
      ...view({ session: SESSION }),
      earlier: [
        memTurn('a', {
          summary: { md: 'Shown directly to you.', kind: 'note', workspaces: [] },
          parts: [
            {
              callId: 'c-00000001',
              workspace: fin,
              sections: [{ id: 'answer', md: 'Finance churn fell.' }],
              notShownToAgent: true,
              folded: true,
            },
          ],
        }),
      ],
      threads: [{ turnId: T('a'), threadId: THREAD_A }],
    });
    const d = el.querySelector('[data-conversation-detail]') as HTMLElement;
    expect(d.querySelector('.agent-part-title')?.textContent).toBe('Answer from Finance');
    expect(d.textContent).toContain('Finance churn fell.');
    expect(d.querySelector('[data-action="fold"]')).toBeNull();
  });
});

describe('the note of a stored turn (ADR 0122)', () => {
  beforeEach(() => tauri.reset());

  it('[owner bug] a link in the stored note opens through link_open with the part of its host', async () => {
    const { el } = await mount();
    const url = 'https://example.cloud.databricks.test/one/chat/threads/t1';
    const other = 'https://elsewhere.example/x';
    const t = stored('a', { summaryMd: `See [the thread](${url}) and [that](${other}).` });
    await feed([thread(THREAD_A, [t])]);
    const note = el.querySelector('[data-agent-note]') as HTMLElement;
    const links = [...note.querySelectorAll<HTMLAnchorElement>('a.md-link')];
    expect(links).toHaveLength(1);
    // A host that is no workspace of the turn stays plain text.
    expect(note.querySelector('.md-link--blocked')).not.toBeNull();
    await click(links[0]);
    expect(payloads(CONVERSATIONS_VIEW_ID)).toContainEqual({
      target: 'link_open',
      url,
      requestId: t.parts[0]?.requestId,
    });
  });

  const noteLink = async (turnOver: Partial<AgentThreadTurn>, url: string) => {
    const { el } = await mount();
    await feed([thread(THREAD_A, [stored('a', { summaryMd: `See [it](${url}).`, ...turnOver })])]);
    const a = el.querySelector<HTMLAnchorElement>('[data-agent-note] a.md-link');
    await click(a);
    return payloads(CONVERSATIONS_VIEW_ID).filter((p) => p['target'] === 'link_open');
  };

  it('[L2] two parts share a host: the part that did not fail speaks for it', async () => {
    const parts = [part(1, { status: 'failed' }), part(2)];
    const url = 'https://example.cloud.databricks.test/x';
    const opens = await noteLink({ parts }, url);
    expect(opens).toEqual([{ target: 'link_open', url, requestId: parts[1]?.requestId }]);
  });

  it('a home host written as https://host and a second host both resolve to their part', async () => {
    const parts = [
      part(1, { workspaceHost: 'https://home.cloud.databricks.test' }),
      part(2, { workspaceHost: 'other.cloud.databricks.test' }),
    ];
    const home = await noteLink({ parts }, 'https://home.cloud.databricks.test/h');
    expect(home).toEqual([
      {
        target: 'link_open',
        url: 'https://home.cloud.databricks.test/h',
        requestId: parts[0]?.requestId,
      },
    ]);
    tauri.reset();
    const other = await noteLink({ parts }, 'https://other.cloud.databricks.test/o');
    expect(other[0]?.['requestId']).toBe(parts[1]?.requestId);
  });
});
