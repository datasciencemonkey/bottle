// @vitest-environment jsdom
/** ADR 0122: the `[n]` markers of the summary are chips that reach the part of a workspace. */
import { act } from 'react';
import type { AgentPartView, AgentTurnView } from '../../shared/ipc';
import { inputEmits, render, tauri } from '../test-tauri';
import { AgentTurnCard, citeLabel, FLASH_MS } from './AgentTurnCard';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const MOG = '00000000-0000-4000-8000-00000000000a';
const T = 'a'.repeat(32);
const sales = { name: 'sales', colorVariant: 'sky' };
const fin = { name: 'finance', colorVariant: 'rose' };
const LINK = 'https://sales.cloud.databricks.test/one/chat/threads/x';
const part = (callId: string, ws: typeof sales, folded?: boolean): AgentPartView => ({
  callId,
  workspace: ws,
  sections: [
    { id: 'answer', md: `Answer of ${ws.name}.` },
    { id: 'footer', deepLink: LINK, workspaceHost: 'sales.cloud.databricks.test' },
  ],
  ...(folded ? { folded } : {}),
});
const turn = (md: string, citations: unknown[], o: Partial<AgentTurnView> = {}): AgentTurnView =>
  ({
    turnId: T,
    question: 'Compare?',
    state: 'done',
    used: 2,
    budget: 10,
    agentWorkspace: sales,
    calls: [],
    parts: [part('c-1', sales), part('c-2', fin)],
    summary: { md, kind: 'combined', workspaces: [sales, fin], citations },
    ...o,
  }) as AgentTurnView;
const cites = [
  { n: 1, callId: 'c-1', workspace: sales, deepLink: LINK },
  { n: 2, callId: 'c-2', workspace: fin },
];
const click = (el: Element | null, init: MouseEventInit = {}) =>
  act(async () => {
    (el as HTMLElement).dispatchEvent(
      new MouseEvent('click', { bubbles: true, cancelable: true, ...init }),
    );
  });
const chips = [{ id: `agent-open:${T}`, label: 'Open in Conversations' }];
const card = (t: AgentTurnView) => <AgentTurnCard mogId={MOG} turn={t} chips={chips} />;
const note = (el: HTMLElement) => el.querySelector('[data-agent-note]') as HTMLElement;

beforeEach(() => tauri.reset());
afterEach(() => {
  document.body.innerHTML = '';
  vi.useRealTimers();
});

describe('citation markers', () => {
  it('draws each known [n] as a chip with an aria-label, and leaves other text alone', async () => {
    const { el } = await render(card(turn('Sales [1] and finance [2][1], not [3] or [x].', cites)));
    const marks = [...note(el).querySelectorAll<HTMLElement>('[data-agent-cite]')];
    expect(marks.map((m) => m.textContent)).toEqual(['1', '2', '1']);
    expect(marks.map((m) => m.getAttribute('aria-label'))).toEqual([
      citeLabel(1, 'sales'),
      'Source 2: finance',
      'Source 1: sales',
    ]);
    expect(marks[0]?.tagName).toBe('BUTTON');
    expect(note(el).textContent).toContain('not [3] or [x].');
  });

  it('draws nothing for a summary without citations', async () => {
    const { el } = await render(card(turn('Sales [1].', [])));
    expect(el.querySelector('[data-agent-cite]')).toBeNull();
    expect(note(el).textContent).toContain('Sales [1].');
  });

  it('puts no raw HTML in the page from a marker or a bad citation', async () => {
    const evil = {
      n: 1,
      callId: 'c-1',
      workspace: { name: '<img src=x onerror=alert(1)>', colorVariant: 'sky' },
    };
    const { el } = await render(card(turn('<b>x</b> [1] <script>1</script>', [evil, { n: 'x' }])));
    expect(el.querySelector('img, script, b')).toBeNull();
    const mark = el.querySelector('[data-agent-cite]') as HTMLElement;
    expect(mark.getAttribute('aria-label')).toBe('Source 1: <img src=x onerror=alert(1)>');
    expect(note(el).textContent).toContain('<b>x</b>');
  });

  it('scrolls to the part and highlights it for a moment, then clears', async () => {
    vi.useFakeTimers();
    const scroll = vi.fn();
    Element.prototype.scrollIntoView = scroll;
    const { el } = await render(card(turn('Sales [1] and finance [2].', cites)));
    await click(el.querySelector('[data-agent-cite="2"]'));
    const target = el.querySelector('[data-agent-part="c-2"]') as HTMLElement;
    expect(scroll).toHaveBeenCalledTimes(1);
    expect(scroll.mock.instances[0]).toBe(target);
    expect(target.hasAttribute('data-flash')).toBe(true);
    await act(async () => void vi.advanceTimersByTime(FLASH_MS + 1));
    expect(target.hasAttribute('data-flash')).toBe(false);
  });

  it('opens the fold that holds the part before it scrolls', async () => {
    Element.prototype.scrollIntoView = vi.fn();
    const t = turn('Sales [1].', cites, {
      parts: [part('c-1', sales, true), part('c-2', fin, true)],
    });
    const { el } = await render(card(t));
    const fold = el.querySelector('#agent-fold-' + T) as HTMLElement;
    expect(fold.hidden).toBe(true);
    await click(el.querySelector('[data-agent-cite="1"]'));
    expect(fold.hidden).toBe(false);
    expect(el.querySelector('[data-agent-part="c-1"]')?.hasAttribute('data-flash')).toBe(true);
    // The user can still close the fold.
    await click(el.querySelector('[data-action="fold"]'));
    expect(fold.hidden).toBe(true);
  });

  it('opens Open in Databricks through link_open, from the part footer and from shift-click', async () => {
    Element.prototype.scrollIntoView = vi.fn();
    const { el } = await render(card(turn('Sales [1].', cites)));
    await click(el.querySelector('[data-agent-cite="1"]'));
    expect(inputEmits()).toEqual([]);
    await click(el.querySelector('[data-agent-cite="1"]'), { shiftKey: true });
    expect(inputEmits()).toContainEqual(
      expect.objectContaining({ kind: 'click', payload: { target: 'link_open', url: LINK } }),
    );
    tauri.invoke.mockClear();
    await click(el.querySelector('[data-agent-part="c-1"] [data-action="open-in-databricks"]'));
    expect(inputEmits()).toContainEqual(
      expect.objectContaining({ kind: 'click', payload: { target: 'link_open', url: LINK } }),
    );
  });

  it('makes the "Combined from" chips reach the answer of their workspace', async () => {
    Element.prototype.scrollIntoView = vi.fn();
    const { el } = await render(card(turn('Both.', [])));
    const group = el.querySelector('[data-agent-note-chips]') as HTMLElement;
    const buttons = group.querySelectorAll('button');
    expect(buttons).toHaveLength(2);
    await click(buttons[1] as HTMLElement);
    expect(el.querySelector('[data-agent-part="c-2"]')?.hasAttribute('data-flash')).toBe(true);
  });
});
