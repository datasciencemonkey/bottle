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
    expect(turnHeadline(view({ state: 'done' }))).toBe('Answered from Sales · 1 of 2 answered');
    const both = view({
      state: 'done',
      calls: view().calls.map((c) => ({ ...c, state: 'ready' as const })),
    });
    expect(turnHeadline(both)).toBe('Answered from Sales and Finance · 2 Genie calls');
    const first = both.calls[0];
    if (!first) throw new Error('no call');
    const more = ['Ops', 'EU'].map((name, n) => ({
      ...first,
      callId: `c-0000000${n + 3}`,
      workspace: { name, colorVariant: 'teal' },
    }));
    const three = { ...both, calls: [...both.calls, ...more] };
    expect(turnHeadline(three)).toBe('Answered from Sales, Finance +2 · 4 Genie calls');
    expect(turnHeadline(view({ state: 'timed_out' }))).toBe('Timed out');
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

  it('[P1-7] a finished card fades for 300 ms after 20 s, then closes; Close closes at once', async () => {
    vi.useFakeTimers();
    const { el } = await render(
      <AgentTurnCard mogId={MOG} turn={view({ state: 'done' })} chips={[]} />,
    );
    const closes = () =>
      inputEmits().filter(
        (e) => (e as { payload?: { target?: string } }).payload?.target === 'card_done',
      );
    const frame = () => el.querySelector('[data-agent-card]')?.getAttribute('data-state');
    await act(async () => void vi.advanceTimersByTime(19_999));
    expect(frame()).toBe('shown');
    await act(async () => void vi.advanceTimersByTime(1));
    expect(frame()).toBe('fading');
    expect(closes()).toHaveLength(0);
    await act(async () => void vi.advanceTimersByTime(299));
    expect(closes()).toHaveLength(0);
    await act(async () => void vi.advanceTimersByTime(1));
    expect(closes()).toHaveLength(1);
    const close = el.querySelector<HTMLButtonElement>('[data-action="dismiss"]');
    expect(close?.getAttribute('aria-label')).toBe('Close');
    await click(close);
    expect(closes()).toHaveLength(2);
  });

  it('[P1-7] a pointer on the card holds it, and after the leave it stays 2.5 s', async () => {
    vi.useFakeTimers();
    const { el } = await render(
      <AgentTurnCard mogId={MOG} turn={view({ state: 'done' })} chips={[]} />,
    );
    const card = el.querySelector('[data-agent-card]') as HTMLElement;
    const closes = () =>
      inputEmits().filter(
        (e) => (e as { payload?: { target?: string } }).payload?.target === 'card_done',
      );
    await act(async () => void card.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })));
    await act(async () => void vi.advanceTimersByTime(60_000));
    expect(closes()).toHaveLength(0);
    await act(async () => void card.dispatchEvent(new MouseEvent('mouseout', { bubbles: true })));
    await act(async () => void vi.advanceTimersByTime(2_499));
    expect(card.getAttribute('data-state')).toBe('shown');
    await act(async () => void vi.advanceTimersByTime(1));
    expect(card.getAttribute('data-state')).toBe('fading');
  });

  it('[P1-7] a needs_you card never fades, and Open Settings is its main action', async () => {
    vi.useFakeTimers();
    const { el } = await render(
      <AgentTurnCard
        mogId={MOG}
        turn={view({ state: 'needs_you', notice: 'Your sign-in expired.' })}
        chips={[
          { id: `agent-settings:${T}`, label: 'Open Settings' },
          { id: `agent-open:${T}`, label: 'Open in Agent' },
        ]}
      />,
    );
    await act(async () => void vi.advanceTimersByTime(120_000));
    expect(inputEmits()).toEqual([]);
    expect(el.querySelector(`[data-chip="agent-settings:${T}"]`)?.className).toContain(
      'popup-action',
    );
    expect(el.querySelector(`[data-chip="agent-open:${T}"]`)?.className).not.toContain(
      'popup-action',
    );
  });

  it('[P0-1] the footer and the actions sit outside the scroll region', async () => {
    const { el } = await render(
      <AgentTurnCard mogId={MOG} turn={view({ tokens: 41_000 })} chips={chips} />,
    );
    const body = el.querySelector('.agent-card-body') as HTMLElement;
    expect(body.querySelector('[data-chip], .agent-foot')).toBeNull();
    expect(el.querySelector('.agent-card-foot [data-chip="agent-open:' + T + '"]')).not.toBeNull();
  });

  it('[P0-2] waiting turns are compact rows under the live one, each with a Remove ×', async () => {
    const q = [{ id: `agent-queued:${'b'.repeat(32)}`, label: 'Later?' }];
    const { el } = await render(
      <AgentTurnCard mogId={MOG} turn={view()} chips={chips} queued={q} />,
    );
    const row = el.querySelector(`[data-agent-queued="${'b'.repeat(32)}"]`);
    expect(row?.textContent).toContain('Queued');
    // The waiting copy shows once above the list, not in each row.
    expect(row?.textContent).not.toContain('Sends when the answer above lands');
    expect(el.querySelectorAll('.agent-queued-copy')).toHaveLength(1);
    expect(el.querySelector('.agent-queued-copy')?.textContent).toBe(
      'Sends when the answer above lands',
    );
    await click(row?.querySelector('button') ?? null);
    expect(inputEmits()).toContainEqual({
      mogId: MOG,
      kind: 'chip_click',
      payload: { chipId: `agent-stop:${'b'.repeat(32)}` },
    });
  });

  it('[P2-12] Remove is a quiet toggle and Stop is the one main button', async () => {
    const stop = await render(<AgentTurnCard mogId={MOG} turn={view()} chips={chips} />);
    expect(stop.el.querySelector(`[data-chip="agent-stop:${T}"]`)?.className).toContain(
      'popup-action',
    );
    const rem = await render(
      <AgentTurnCard
        mogId={MOG}
        turn={view({ state: 'queued', calls: [] })}
        chips={[{ id: `agent-stop:${T}`, label: 'Remove' }]}
      />,
    );
    expect(rem.el.querySelector(`[data-chip="agent-stop:${T}"]`)?.className).not.toContain(
      'popup-action',
    );
  });

  it('[P1-6] the call rows of a finished turn sit behind one toggle', async () => {
    const { el } = await render(
      <AgentTurnCard mogId={MOG} turn={view({ state: 'done' })} chips={chips} />,
    );
    const t = el.querySelector<HTMLButtonElement>('[data-action="calls"]');
    expect(t?.textContent).toBe('Show 2 Genie calls');
    expect(t?.getAttribute('aria-expanded')).toBe('false');
    expect((el.querySelector('#' + t?.getAttribute('aria-controls')) as HTMLElement).hidden).toBe(
      true,
    );
    await click(t);
    expect(t?.textContent).toBe('Hide 2 Genie calls');
  });

  it('[P2-10] an ended turn shows a short state word, and the reason once', async () => {
    const { el } = await render(
      <AgentTurnCard
        mogId={MOG}
        turn={view({ state: 'timed_out', notice: 'The agent timed out. Asked Sales only.' })}
        chips={chips}
      />,
    );
    expect(el.querySelector('[data-agent-headline]')?.textContent).toBe('Timed out');
    expect(el.querySelector('[data-agent-notice]')?.textContent).toBe(
      'The agent timed out. Asked Sales only.',
    );
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

describe('answer first', () => {
  it('an ended turn shows its answer before its call rows; a running turn the other way', async () => {
    const part = {
      callId: 'c-00000001',
      workspace: sales,
      sections: [{ id: 'answer' as const, md: 'Answer one' }],
    };
    const done = await render(
      <AgentTurnCard mogId={MOG} turn={view({ state: 'done', parts: [part] })} chips={chips} />,
    );
    const html = done.el.innerHTML;
    expect(html.indexOf('data-agent-part')).toBeGreaterThan(-1);
    expect(html.indexOf('data-agent-part')).toBeLessThan(html.indexOf('data-action="calls"'));
    const run = await render(<AgentTurnCard mogId={MOG} turn={view()} chips={chips} />);
    expect(run.el.innerHTML.indexOf('data-agent-call')).toBeGreaterThan(-1);
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
