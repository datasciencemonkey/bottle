// @vitest-environment jsdom
/** ADR 0122: the `[n]` markers of the summary are chips that reach the part of a workspace. */
import { act, type ReactNode } from 'react';
import type { Root } from 'react-dom/client';
import { asAgentTurnView } from '../../shared/hub-agent';
import type { AgentPartView, AgentTurnView } from '../../shared/ipc';
import { inputEmits, render as renderNode, tauri } from '../test-tauri';
import { AgentTurnCard, CITE_HINT, FLASH_MS } from './AgentTurnCard';

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
    parts: [part('c-00000001', sales), part('c-00000002', fin)],
    summary: { md, kind: 'combined', workspaces: [sales, fin], citations },
    ...o,
  }) as AgentTurnView;
const cites = [
  {
    n: 1,
    callId: 'c-00000001',
    workspace: sales,
    deepLink: LINK,
    workspaceHost: 'sales.cloud.databricks.test',
  },
  { n: 2, callId: 'c-00000002', workspace: fin },
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

const roots: Root[] = [];
const render = async (node: ReactNode) => {
  const r = await renderNode(node);
  roots.push(r.root);
  return r;
};
const scroll = vi.fn();
const original = Object.getOwnPropertyDescriptor(Element.prototype, 'scrollIntoView');
beforeEach(() => {
  tauri.reset();
  scroll.mockClear();
  Element.prototype.scrollIntoView = scroll;
});
afterEach(async () => {
  for (const r of roots.splice(0)) await act(async () => r.unmount());
  document.body.innerHTML = '';
  vi.useRealTimers();
  // Put back what the file replaced: jsdom has no scrollIntoView.
  if (original) Object.defineProperty(Element.prototype, 'scrollIntoView', original);
  else delete (Element.prototype as { scrollIntoView?: unknown }).scrollIntoView;
});

describe('citation markers', () => {
  it('draws each known [n] as a chip with an aria-label, and leaves other text alone', async () => {
    const { el } = await render(card(turn('Sales [1] and finance [2][1], not [3] or [x].', cites)));
    const marks = [...note(el).querySelectorAll<HTMLElement>('[data-agent-cite]')];
    expect(marks.map((m) => m.textContent)).toEqual(['1', '2', '1']);
    expect(marks.map((m) => m.getAttribute('aria-label'))).toEqual([
      'Source 1: sales',
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
      callId: 'c-00000001',
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
    const { el } = await render(card(turn('Sales [1] and finance [2].', cites)));
    await click(el.querySelector('[data-agent-cite="2"]'));
    const target = el.querySelector('[data-agent-part="c-00000002"]') as HTMLElement;
    expect(scroll).toHaveBeenCalledTimes(1);
    expect(scroll.mock.instances[0]).toBe(target);
    expect(target.hasAttribute('data-flash')).toBe(true);
    await act(async () => void vi.advanceTimersByTime(FLASH_MS + 1));
    expect(target.hasAttribute('data-flash')).toBe(false);
  });

  it('opens the fold that holds the part before it scrolls', async () => {
    const t = turn('Sales [1].', cites, {
      parts: [part('c-00000001', sales, true), part('c-00000002', fin, true)],
    });
    const { el } = await render(card(t));
    const fold = el.querySelector('#agent-fold-' + T) as HTMLElement;
    expect(fold.hidden).toBe(true);
    await click(el.querySelector('[data-agent-cite="1"]'));
    expect(fold.hidden).toBe(false);
    expect(el.querySelector('[data-agent-part="c-00000001"]')?.hasAttribute('data-flash')).toBe(
      true,
    );
    // The user can still close the fold.
    await click(el.querySelector('[data-action="fold"]'));
    expect(fold.hidden).toBe(true);
  });

  it('opens Open in Databricks through link_open, from the part footer and from shift-click', async () => {
    const { el } = await render(card(turn('Sales [1].', cites)));
    await click(el.querySelector('[data-agent-cite="1"]'));
    expect(inputEmits()).toEqual([]);
    await click(el.querySelector('[data-agent-cite="1"]'), { shiftKey: true });
    expect(inputEmits()).toContainEqual(
      expect.objectContaining({ kind: 'click', payload: { target: 'link_open', url: LINK } }),
    );
    tauri.invoke.mockClear();
    await click(
      el.querySelector('[data-agent-part="c-00000001"] [data-action="open-in-databricks"]'),
    );
    expect(inputEmits()).toContainEqual(
      expect.objectContaining({ kind: 'click', payload: { target: 'link_open', url: LINK } }),
    );
  });

  it('makes the "Combined from" chips reach the answer of their workspace', async () => {
    const { el } = await render(card(turn('Both.', [])));
    const group = el.querySelector('[data-agent-note-chips]') as HTMLElement;
    const buttons = group.querySelectorAll('button');
    expect(buttons).toHaveLength(2);
    await click(buttons[1] as HTMLElement);
    expect(el.querySelector('[data-agent-part="c-00000002"]')?.hasAttribute('data-flash')).toBe(
      true,
    );
  });

  it('[M1] citations pass through asAgentTurnView into the view', async () => {
    const wire = JSON.parse(JSON.stringify(turn('Sales [1] and finance [2].', cites)));
    const checked = asAgentTurnView(wire);
    expect(checked?.summary?.citations).toHaveLength(2);
    const { el } = await render(card(checked as AgentTurnView));
    expect(el.querySelectorAll('[data-agent-cite]')).toHaveLength(2);
  });

  it('[L1] a link of another host, or with no host, is no link', async () => {
    const bad = [
      { n: 1, callId: 'c-00000001', workspace: sales, deepLink: LINK },
      {
        n: 2,
        callId: 'c-00000002',
        workspace: fin,
        deepLink: 'https://evil.example/x',
        workspaceHost: 'sales.cloud.databricks.test',
      },
    ];
    const { el } = await render(card(turn('Sales [1] and finance [2].', bad)));
    for (const n of [1, 2]) {
      const chip = el.querySelector(`[data-agent-cite="${n}"]`) as HTMLElement;
      expect(chip.hasAttribute('aria-describedby'), String(n)).toBe(false);
    }
  });

  it('[M2] a chip with a link says how to open it, and Shift+Enter opens it once', async () => {
    const { el } = await render(card(turn('Sales [1] and finance [2].', cites)));
    const [one, two] = [1, 2].map(
      (n) => el.querySelector(`[data-agent-cite="${n}"]`) as HTMLElement,
    );
    const hint = document.getElementById(one?.getAttribute('aria-describedby') ?? '');
    expect(hint?.textContent).toBe(CITE_HINT);
    // No link, no hint.
    expect(two?.hasAttribute('aria-describedby')).toBe(false);
    const ev = new KeyboardEvent('keydown', {
      key: 'Enter',
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    });
    await act(async () => void one?.dispatchEvent(ev));
    expect(ev.defaultPrevented).toBe(true);
    const opens = inputEmits().filter(
      (e) => (e as { payload?: { target?: string } }).payload?.target === 'link_open',
    );
    expect(opens).toHaveLength(1);
    // A plain Enter is left to the button: it reaches the part.
    const plain = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
    await act(async () => void one?.dispatchEvent(plain));
    expect(plain.defaultPrevented).toBe(false);
  });

  it('[L3] moves focus to the part it reached', async () => {
    const { el } = await render(card(turn('Finance [2].', cites)));
    await click(el.querySelector('[data-agent-cite="2"]'));
    const target = el.querySelector('[data-agent-part="c-00000002"]') as HTMLElement;
    expect(target.getAttribute('tabindex')).toBe('-1');
    expect(document.activeElement).toBe(target);
  });

  it('[L4] a repeat click restarts the highlight', async () => {
    vi.useFakeTimers();
    const { el } = await render(card(turn('Finance [2].', cites)));
    const mark = el.querySelector('[data-agent-cite="2"]');
    await click(mark);
    const first = el.querySelector('[data-agent-part="c-00000002"]')?.getAttribute('data-flash');
    await click(mark);
    const second = el.querySelector('[data-agent-part="c-00000002"]')?.getAttribute('data-flash');
    expect(first).not.toBeNull();
    expect(second).not.toBeNull();
    expect(second).not.toBe(first);
  });
});
