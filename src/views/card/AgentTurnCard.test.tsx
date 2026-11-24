// @vitest-environment jsdom
/** Slice 2a: the card of an agent turn, and its body (shared with the hub panel). Fake values. */
import { act } from 'react';
import type { AgentTurnView } from '../../shared/ipc';
import { inputEmits, render, tauri } from '../test-tauri';
import {
  AgentTurnCard,
  callLabel,
  clock,
  tokensText,
  turnHeadline,
  FROM_EARLIER_COPY,
  NOT_SHOWN_COPY,
} from './AgentTurnCard';
import { CardView } from './CardView';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const MOG = '00000000-0000-4000-8000-00000000000a';
const T = 'a'.repeat(32);
const sales = { name: 'Sales', colorVariant: 'sky' };
const fin = { name: 'Finance', colorVariant: 'rose' };
const view = (o: Partial<AgentTurnView> = {}): AgentTurnView => ({
  turnId: T,
  question: 'Compare churn?',
  state: 'running',
  used: 3,
  budget: 10,
  agentWorkspace: sales,
  calls: [
    { callId: 'c-00000001', workspace: sales, question: 'Churn in sales?', state: 'ready' },
    {
      callId: 'c-00000002',
      workspace: fin,
      question: 'Churn in finance?',
      state: 'asking',
      startedAt: 1_000,
    },
  ],
  parts: [],
  ...o,
});
const chips = [
  { id: `agent-stop:${T}`, label: 'Stop' },
  { id: `agent-open:${T}`, label: 'Open in Agent' },
];
const click = (el: Element | null) => act(async () => (el as HTMLElement).click());

beforeEach(() => tauri.reset());
afterEach(() => {
  document.body.innerHTML = '';
  vi.useRealTimers();
});

describe('agent turn text', () => {
  it('writes the header of each state', () => {
    expect(turnHeadline(view({ state: 'planning', calls: [] }))).toBe(
      'Planning with Genie Code in Sales…',
    );
    expect(turnHeadline(view())).toBe('Asking 2 workspaces · 3 of 10 Genie calls');
    expect(turnHeadline(view({ state: 'done' }))).toBe('Asked 2 workspaces · 3 Genie calls');
    expect(turnHeadline(view({ state: 'timed_out' }))).toBe('The agent timed out.');
    expect(tokensText(41_200)).toBe('41k tokens');
    expect(tokensText(850)).toBe('850 tokens');
    expect(clock(12_000)).toBe('0:12');
    const asking = view().calls[1];
    if (asking) expect(callLabel(asking, 13_000)).toBe('Asking Finance… 0:12');
  });
});

describe('AgentTurnCard', () => {
  it('shows one row for each call, "n of N", Stop and Open in Agent', async () => {
    const { el } = await render(<AgentTurnCard mogId={MOG} turn={view()} chips={chips} />);
    expect(el.querySelectorAll('[data-agent-call]')).toHaveLength(2);
    expect(el.querySelector('[data-agent-headline]')?.textContent).toContain('3 of 10');
    expect(el.querySelector('[data-agent-call="c-00000001"]')?.textContent).toContain('Ready');
    expect(
      el
        .querySelector('[data-agent-call="c-00000002"] .agent-call-question')
        ?.getAttribute('title'),
    ).toBe('Churn in finance?');
    await click(el.querySelector(`[data-chip="agent-stop:${T}"]`));
    expect(inputEmits()).toContainEqual({
      mogId: MOG,
      kind: 'chip_click',
      payload: { chipId: `agent-stop:${T}` },
    });
  });

  it('[AC-2a.10] shows "Still working…" in the slow state, in a live region', async () => {
    const { el } = await render(
      <AgentTurnCard mogId={MOG} turn={view({ state: 'slow' })} chips={chips} />,
    );
    const head = el.querySelector('[data-agent-headline]');
    expect(head?.textContent).toContain('Still working…');
    expect(head?.getAttribute('aria-live')).toBe('polite');
  });

  it('[AC-2a.19] a queued turn shows the Queued pill, the waiting copy and Remove', async () => {
    const { el } = await render(
      <AgentTurnCard
        mogId={MOG}
        turn={view({ state: 'queued', calls: [], position: 1 })}
        chips={[{ id: `agent-stop:${T}`, label: 'Remove' }]}
      />,
    );
    expect(el.querySelector('.agent-pill')?.textContent).toBe('Queued');
    expect(el.textContent).toContain('Sends when the answer above lands');
    expect(el.querySelector(`[data-chip="agent-stop:${T}"]`)?.textContent).toBe('Remove');
  });

  it('shows the note, the cited part, a fold for the rest, and the earlier tag', async () => {
    const part = (id: string, o = {}) => ({
      callId: id,
      workspace: fin,
      sections: [{ id: 'answer' as const, md: `Answer ${id}` }],
      ...o,
    });
    const t = view({
      state: 'done',
      tokens: 41_000,
      summary: { md: 'Finance is higher.', kind: 'note', workspaces: [fin] },
      parts: [
        part('c-00000001'),
        part('c-00000003', { folded: true }),
        part('c-00000004', { fromEarlier: true, notShownToAgent: true }),
      ],
    });
    const { el } = await render(<AgentTurnCard mogId={MOG} turn={t} chips={chips} />);
    expect(el.querySelector('[data-agent-note]')?.textContent).toContain('Note from your MOG');
    expect(el.querySelector('[data-agent-footer]')?.textContent).toBe(
      'Planned by Genie Code in Sales · 41k tokens',
    );
    expect(el.textContent).toContain(FROM_EARLIER_COPY);
    expect(el.textContent).toContain(NOT_SHOWN_COPY);
    const fold = el.querySelector<HTMLButtonElement>('[data-action="fold"]');
    expect(fold?.textContent).toBe('1 more answer');
    expect(fold?.getAttribute('aria-expanded')).toBe('false');
    await click(fold);
    expect(fold?.getAttribute('aria-expanded')).toBe('true');
  });

  it('Close sends card_done; a finished card fades after 20 s, and a hold stops that', async () => {
    vi.useFakeTimers();
    const { el } = await render(
      <AgentTurnCard mogId={MOG} turn={view({ state: 'done' })} chips={[]} />,
    );
    const closes = () =>
      inputEmits().filter(
        (e) => (e as { payload?: { target?: string } }).payload?.target === 'card_done',
      );
    await act(async () => void vi.advanceTimersByTime(19_999));
    expect(closes()).toHaveLength(0);
    await act(async () => void vi.advanceTimersByTime(1));
    expect(closes()).toHaveLength(1);
    const close = el.querySelector<HTMLButtonElement>('[data-action="dismiss"]');
    expect(close?.getAttribute('aria-label')).toBe('Close');
    await click(close);
    expect(closes()).toHaveLength(2);
  });

  it('a running card never fades', async () => {
    vi.useFakeTimers();
    await render(<AgentTurnCard mogId={MOG} turn={view()} chips={chips} />);
    await act(async () => void vi.advanceTimersByTime(120_000));
    expect(inputEmits()).toEqual([]);
  });

  it('the scroll body can take the keyboard focus', async () => {
    const { el } = await render(<AgentTurnCard mogId={MOG} turn={view()} chips={chips} />);
    expect(el.querySelector('.agent-card-body')?.getAttribute('tabindex')).toBe('0');
  });
});

describe('CardView with an agent turn', () => {
  it('draws the agent card with the agent chips only', async () => {
    const { el } = await render(<CardView mogId={MOG} />);
    await tauri.emit('card:update', {
      mogId: MOG,
      blurbs: [{ id: 'ask:u1', text: '"Compare churn?"', completed: true }],
      chips: [
        ...chips,
        { id: 'copy-x', label: 'Copy' },
        { id: 'agent-more:op-old', label: 'Ask other workspaces too' },
      ],
      agentTurn: view(),
    });
    expect(el.querySelector('[data-agent-card]')).not.toBeNull();
    expect([...el.querySelectorAll('[data-chip]')].map((b) => b.getAttribute('data-chip'))).toEqual(
      chips.map((c) => c.id),
    );
    // A bad turn payload shows no agent card.
    await tauri.emit('card:update', {
      mogId: MOG,
      blurbs: [],
      chips: [],
      agentTurn: { turnId: 'x' },
    });
    expect(el.querySelector('[data-agent-card]')).toBeNull();
  });
});
