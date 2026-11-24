// @vitest-environment jsdom
/** Slice 2a: the card of an agent turn, and its body (shared with the hub panel). Fake values. */
import { act } from 'react';
import type { AgentTurnView } from '../../shared/ipc';
import { inputEmits, render, tauri } from '../test-tauri';
import {
  AgentTurnCard,
  callLabel,
  clock,
  firstAnswerLine,
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
  { id: `agent-open:${T}`, label: 'Open in Conversations' },
];
const click = (el: Element | null) => act(async () => (el as HTMLElement).click());

beforeEach(() => tauri.reset());
afterEach(() => {
  document.body.innerHTML = '';
  vi.useRealTimers();
});

describe('agent turn text', () => {
  it('writes the header of each state', () => {
    expect(turnHeadline(view({ state: 'planning', calls: [] }))).toBe('Planning in Sales…');
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
  it('shows one row for each call, "n of N", Stop and Open in Conversations', async () => {
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

  const partOf = (id: string, o = {}) => ({
    callId: id,
    workspace: fin,
    sections: [{ id: 'answer' as const, md: `Answer ${id}` }],
    ...o,
  });

  it('[AC-2b.3] with a combined note: "Combined by your MOG", the source chips, the parts folded under it', async () => {
    const t = view({
      state: 'done',
      tokens: 41_000,
      summary: { md: 'Finance is higher.', kind: 'combined', workspaces: [fin, sales] },
      parts: [
        partOf('c-00000001'),
        partOf('c-00000003', { workspace: sales }),
        partOf('c-00000004', { fromEarlier: true }),
      ],
    });
    const { el } = await render(<AgentTurnCard mogId={MOG} turn={t} chips={chips} />);
    const note = el.querySelector('[data-agent-note]') as HTMLElement;
    expect(note.querySelector('.agent-note-label')).toBeNull();
    expect(note.textContent).not.toContain('Note from your MOG');
    expect(note.textContent).not.toContain('Combined by your MOG');
    expect(note.className).not.toContain('agent-note--after');
    const chipNames = [...note.querySelectorAll('[data-agent-chip]')].map((n) => n.textContent);
    expect(chipNames).toEqual(['Finance', 'Sales']);
    // The note comes before the parts, and every part is folded under one toggle.
    const parts = el.querySelector('.agent-parts') as HTMLElement;
    expect(note.compareDocumentPosition(parts) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(parts.hidden).toBe(true);
    expect(el.querySelector('.agent-part-title')).toBeNull();
    expect(el.querySelector('[data-agent-footer]')?.textContent).toBe(
      'Planned in Sales · 41k tokens',
    );
    const fold = el.querySelector<HTMLButtonElement>('[data-action="fold"]');
    expect(fold?.textContent).toBe('3 workspace answers');
    expect(fold?.getAttribute('aria-expanded')).toBe('false');
    await click(fold);
    expect(fold?.getAttribute('aria-expanded')).toBe('true');
    expect(fold?.textContent).toBe('Hide workspace answers');
    expect(fold?.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
    expect(parts.hidden).toBe(false);
    expect(el.textContent).toContain(FROM_EARLIER_COPY);
  });

  it('[AC-2b.2] a withheld part under a combined note says "Not shown to the agent" and hides its text', async () => {
    const t = view({
      state: 'done',
      summary: { md: 'Finance is higher.', kind: 'combined', workspaces: [fin] },
      parts: [
        partOf('c-00000001'),
        // The brain swaps a withheld answer for this fixed text (agent-card.test.ts checks that
        // the raw text never reaches the part).
        partOf('c-00000002', {
          workspace: sales,
          notShownToAgent: true,
          sections: [{ id: 'answer' as const, md: 'This answer was held back.' }],
        }),
      ],
    });
    const { el } = await render(<AgentTurnCard mogId={MOG} turn={t} chips={chips} />);
    // The note still comes first; the withheld part is in view, the rest is folded.
    expect(el.querySelector('[data-agent-note]')?.className).not.toContain('agent-note--after');
    const withheld = el.querySelector('[data-agent-part="c-00000002"]') as HTMLElement;
    expect(withheld.closest('[hidden]')).toBeNull();
    expect(withheld.textContent).toContain(NOT_SHOWN_COPY);
    expect(withheld.textContent).toContain('This answer was held back.');
    const folded = el.querySelector('[data-agent-part="c-00000001"]') as HTMLElement;
    expect(folded.closest('[hidden]')).not.toBeNull();
    expect(el.querySelector('[data-action="fold"]')?.textContent).toBe('1 workspace answer');
  });

  it('a combined note whose parts were all withheld has no fold, and the parts stay in view', async () => {
    const t = view({
      state: 'done',
      summary: { md: 'Finance is higher.', kind: 'combined', workspaces: [fin] },
      parts: [
        partOf('c-00000001', { notShownToAgent: true }),
        partOf('c-00000002', { workspace: sales, notShownToAgent: true }),
      ],
    });
    const { el } = await render(<AgentTurnCard mogId={MOG} turn={t} chips={chips} />);
    expect(el.querySelector('[data-action="fold"]')).toBeNull();
    for (const id of ['c-00000001', 'c-00000002']) {
      const p = el.querySelector(`[data-agent-part="${id}"]`) as HTMLElement;
      expect(p.closest('[hidden]')).toBeNull();
      expect(p.textContent).toContain(NOT_SHOWN_COPY);
    }
    expect(el.querySelector('[data-agent-note]')?.className).not.toContain('agent-note--after');
  });

  it('names the list of a combined fold "Workspace answers"', async () => {
    const t = view({
      state: 'done',
      summary: { md: 'Finance is higher.', kind: 'combined', workspaces: [fin, sales] },
      parts: [partOf('c-00000001'), partOf('c-00000002', { workspace: sales })],
    });
    const { el } = await render(<AgentTurnCard mogId={MOG} turn={t} chips={chips} />);
    expect(el.querySelector('[data-agent-note-chips]')?.getAttribute('role')).toBe('group');
    const fold = el.querySelector('[data-action="fold"]');
    const list = document.getElementById(fold?.getAttribute('aria-controls') ?? '');
    expect(list?.getAttribute('aria-label')).toBe('Workspace answers');
  });

  it('[owner bug] a summary is the main answer: markdown, a trusted link, Open in Databricks, chips, no "Note from your MOG"', async () => {
    const host = 'sales.cloud.databricks.test';
    const link = `https://${host}/one/chat/threads/x`;
    const t = view({
      state: 'done',
      summary: {
        md: `## Revenue\n\n**Up 4%** [1]. See [the dashboard](https://${host}/sql/dashboards/d1) or [evil](https://evil.example/x).`,
        kind: 'note',
        workspaces: [sales],
        citations: [
          { n: 1, callId: 'c-00000001', workspace: sales, deepLink: link, workspaceHost: host },
        ],
      },
      parts: [partOf('c-00000001', { workspace: sales }), partOf('c-00000002')],
    });
    const { el } = await render(<AgentTurnCard mogId={MOG} turn={t} chips={chips} />);
    const main = el.querySelector('[data-agent-summary]') as HTMLElement;
    expect(main.textContent).not.toContain('Note from your MOG');
    expect(main.querySelector('strong')?.textContent).toBe('Revenue');
    expect([...main.querySelectorAll('strong')].map((s) => s.textContent)).toContain('Up 4%');
    expect(main.querySelector('[data-agent-cite="1"]')).not.toBeNull();
    const anchors = [...main.querySelectorAll<HTMLAnchorElement>('a')];
    expect(anchors.map((a) => a.getAttribute('href'))).toContain(
      `https://${host}/sql/dashboards/d1`,
    );
    expect(anchors.map((a) => a.getAttribute('href'))).not.toContain('https://evil.example/x');
    const open = main.querySelector('[data-action="open-in-databricks"]');
    expect(open?.textContent).toBe('Open in Databricks');
    // The answer comes first; the part folds under it.
    expect(el.querySelector('.agent-part-title')).toBeNull();
    expect(el.querySelector('[data-action="fold"]')?.textContent).toBe('2 workspace answers');
    expect(el.querySelector('.agent-note-label')).toBeNull();
  });

  it('[owner rule] exactly one answered call shows that Genie answer as it is, not the text of the model', async () => {
    const t = view({
      state: 'done',
      summary: { md: 'MODEL-PARAPHRASE', kind: 'note', workspaces: [sales] },
      parts: [partOf('c-00000001', { workspace: sales })],
    });
    const { el } = await render(<AgentTurnCard mogId={MOG} turn={t} chips={chips} />);
    expect(el.textContent).not.toContain('MODEL-PARAPHRASE');
    expect(el.querySelector('[data-agent-summary]')).toBeNull();
    expect(el.querySelector('.agent-part-title')?.textContent).toBe('Answer from Sales');
    expect(el.textContent).toContain('Answer c-00000001');
    expect(el.querySelector('[data-action="fold"]')).toBeNull();
  });

  it('[owner rule] two answered calls show the combined answer first and fold both answers', async () => {
    const t = view({
      state: 'done',
      summary: { md: 'COMBINED-TEXT', kind: 'combined', workspaces: [fin, sales] },
      parts: [partOf('c-00000001'), partOf('c-00000002', { workspace: sales })],
    });
    const { el } = await render(<AgentTurnCard mogId={MOG} turn={t} chips={chips} />);
    expect(el.querySelector('[data-agent-summary]')?.textContent).toContain('COMBINED-TEXT');
    expect(el.querySelector('[data-action="fold"]')?.textContent).toBe('2 workspace answers');
  });

  it('[owner bug] with no summary text the workspace answers come first, open, each under "Answer from <workspace>"', async () => {
    const t = view({
      state: 'done',
      summary: { md: '', kind: 'note', workspaces: [] },
      parts: [
        partOf('c-00000001', { workspace: sales }),
        // No source cites it, so it was folded before: it shows now, and it says it is not the agent's.
        partOf('c-00000003', { folded: true, notShownToAgent: true }),
      ],
    });
    const { el } = await render(<AgentTurnCard mogId={MOG} turn={t} chips={chips} />);
    const titles = [...el.querySelectorAll('.agent-part-title')].map((n) => n.textContent);
    expect(titles).toEqual(['Answer from Sales', 'Answer from Finance']);
    expect(el.textContent).toContain('Answer c-00000001');
    expect(el.textContent).toContain('Answer c-00000003');
    // No fold hides an answer, and the parts list is not hidden.
    expect(el.querySelector('[data-action="fold"]')).toBeNull();
    expect((el.querySelector('.agent-parts') as HTMLElement).hidden).toBe(false);
    // An empty summary draws no box at all.
    expect(el.querySelector('[data-agent-note]')).toBeNull();
    expect(el.textContent).toContain(NOT_SHOWN_COPY);
  });

  it('[owner bug] with no summary at all the answers show open and first', async () => {
    const t = view({
      state: 'done',
      parts: [partOf('c-00000001', { workspace: sales, folded: true })],
    });
    const { el } = await render(<AgentTurnCard mogId={MOG} turn={t} chips={chips} />);
    expect(el.querySelector('.agent-part-title')?.textContent).toBe('Answer from Sales');
    expect(el.querySelector('[data-agent-note]')).toBeNull();
    expect((el.querySelector('.agent-parts') as HTMLElement).hidden).toBe(false);
  });

  it('firstAnswerLine takes the first line of the first answer, never the note', () => {
    const t = view({
      state: 'done',
      summary: { md: 'Its findings were shown directly to you.', kind: 'note', workspaces: [] },
      parts: [
        { callId: 'c-00000001', workspace: fin, sections: [] },
        partOf('c-00000002', {
          sections: [{ id: 'answer' as const, md: '\n## Churn fell 4%\nMore detail.' }],
        }),
      ],
    });
    expect(firstAnswerLine(t)).toBe('Churn fell 4%');
    expect(firstAnswerLine(view())).toBe('');
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
          { id: `agent-open:${T}`, label: 'Open in Conversations' },
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
