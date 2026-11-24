// @vitest-environment jsdom
/** #573: the Ask Genie status pill and the transient answer card beside the MOG. */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act } from 'react';
import type { Root } from 'react-dom/client';
import {
  AFTER_HOVER_MS,
  ANSWER_READY,
  FAILURE_SHOW_MS,
  answerShowMs,
} from '../../shared/genie-status';
import { FADE_AFTER_MS, FADE_OUT_MS } from './Popup';
import { inputEmits, render, tauri } from '../test-tauri';
import type { CardUpdate } from '../view-ipc';
import { CardView } from './CardView';
import {
  ANSWER_CLAMP_PX,
  CLOSE_LABEL,
  DISMISS_FADE_MS,
  FADE_MS,
  FAILURE_TITLE,
  OPEN_IN_BOTTLE_LABEL,
  SHOW_LESS_LABEL,
  SHOW_MORE_LABEL,
} from './GenieCard';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const ID = '00000000-0000-4000-8000-000000000001';
const OP = '00000000-0000-4000-8000-00000000c573';
const G = `genie:${OP}`;
const ask = { id: 'ask:u1', text: '"What tables can I see?"', pinned: false, completed: true };

const working = (text: string) => ({
  mogId: ID,
  blurbs: [{ id: G, text, pinned: false, completed: false }, ask],
  chips: [],
});
const ANSWER_MD = '## Tables\n\nYou can see about **8,800 tables** in **150 catalogs**.';
const answered = {
  mogId: ID,
  blurbs: [
    {
      id: G,
      text: 'You can see about **8,800 tables**.',
      pinned: false,
      completed: true,
    },
    ask,
  ],
  chips: [],
  sections: [
    { id: 'answer', md: ANSWER_MD },
    { id: 'keyNumbers', items: ['8,800 tables', '150 catalogs'] },
    { id: 'footer' },
    { id: 'dataFlow', text: 'Your question went to Genie One in your workspace.' },
  ],
};
const failed = {
  mogId: ID,
  blurbs: [
    {
      id: G,
      text: 'Genie could not answer this. Try again.\nCode GENIE_FAILED',
      pinned: false,
      completed: true,
    },
    ask,
  ],
  chips: [{ id: `retry:${G}`, label: 'Try again' }],
};

const workspaceFailed = {
  ...failed,
  blurbs: [
    {
      id: G,
      text: "logfood (logfood.cloud.databricks.com) didn't answer. Are you on the VPN?",
      pinned: false,
      completed: true,
    },
    ask,
  ],
  chips: [
    { id: `retry:${G}`, label: 'Try again' },
    { id: `workspace:${G}`, label: 'Change workspace…' },
  ],
};

let roots: Root[] = [];
async function mount() {
  tauri.label = `card-${ID}`;
  const r = await render(<CardView mogId={ID} />);
  roots.push(r.root);
  return r.el;
}
const advance = (ms: number) => act(async () => void vi.advanceTimersByTime(ms));
const clicks = () =>
  inputEmits().filter((e) => (e as { kind: string }).kind === 'click') as {
    payload: { target: string; requestId?: string };
  }[];
const done = () => clicks().filter((c) => c.payload.target === 'card_done').length;
const mouse = (el: Element, type: string) =>
  act(async () => void el.dispatchEvent(new MouseEvent(type, { bubbles: true })));

describe('Genie card (#573)', () => {
  beforeEach(() => {
    tauri.reset();
    vi.useFakeTimers();
  });
  afterEach(async () => {
    await act(async () => roots.forEach((r) => r.unmount()));
    roots = [];
    vi.useRealTimers();
    tauri.label = 'hub';
  });

  it('[G-44] [G-17a] the status pill shows the stage, then the timer, and never fades while Genie works', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', working('Asking Genie'));
    const pill = el.querySelector('[data-genie="working"]');
    expect(pill?.querySelector('[data-genie-stage]')?.textContent).toBe('Asking Genie');
    // One stable live region reads the stage; the shown line and the timer are hidden from it.
    expect(pill?.querySelector('[data-genie-live]')?.getAttribute('aria-live')).toBe('polite');
    expect(pill?.querySelector('[data-genie-live]')?.textContent).toBe('Asking Genie');
    expect(pill?.querySelector('[data-genie-stage]')?.getAttribute('aria-hidden')).toBe('true');
    expect(pill?.querySelectorAll('[aria-live]')).toHaveLength(1);
    expect(pill?.querySelector('[data-genie-timer]')).toBeNull();
    await advance(3_000);
    expect(pill?.querySelector('[data-genie-timer]')?.textContent).toBe('0:03');
    expect(pill?.querySelector('[data-genie-timer]')?.getAttribute('aria-hidden')).toBe('true');
    await advance(61_000);
    expect(el.querySelector('[data-genie-timer]')?.textContent).toBe('Working… 1:04');
    expect(el.querySelector('[data-genie="working"]')).not.toBeNull();
    expect(done()).toBe(0);
  });

  it('[ui-4] labels the working pill and the answer with the target MOG name and colour', async () => {
    const el = await mount();
    // Before the brain answers with the identity, the card shows no label.
    await tauri.emitTo(`card-${ID}`, 'card:update', working('Asking Genie'));
    expect(el.querySelector('.genie-identity')).toBeNull();
    await tauri.emitTo(`card-${ID}`, 'mog:target', {
      mogId: ID,
      name: 'Sales',
      target: true,
      toast: false,
      colorVariant: 'teal',
    });
    const working1 = el.querySelector('[data-genie="working"]') as HTMLElement;
    expect(working1.querySelector('.genie-identity')?.textContent).toBe('Sales');
    expect(working1.style.getPropertyValue('--mog-id-color')).toBe('#0f6b66');
    // [#603 review, finding 5] the name itself is neutral text (AA on the card surface); only
    // the dot, not the text, carries the accent colour.
    expect(working1.querySelector('.genie-identity-dot')).not.toBeNull();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    const answer = el.querySelector('[data-genie="answer"]') as HTMLElement;
    expect(answer.querySelector('.genie-identity')?.textContent).toBe('Sales');
    expect(answer.style.getPropertyValue('--mog-id-color')).toBe('#0f6b66');
    expect(answer.querySelector('.genie-identity-dot')).not.toBeNull();
  });

  it('[G-17b] a new stage replaces the old one; the old line leaves hidden from a screen reader', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', working('Asking Genie'));
    await tauri.emitTo(`card-${ID}`, 'card:update', working('Running the query'));
    expect(el.querySelector('[data-genie-stage]')?.textContent).toBe('Running the query');
    // The same live node gets the new text.
    expect(el.querySelectorAll('[data-genie-live]')).toHaveLength(1);
    expect(el.querySelector('[data-genie-live]')?.textContent).toBe('Running the query');
    expect(el.querySelector('[data-leaving]')?.getAttribute('aria-hidden')).toBe('true');
    await advance(200);
    expect(el.querySelector('[data-leaving]')).toBeNull();
  });

  it('[UX-14a] [SEC-25a] the answer card shows the question, the safe markdown answer and the key numbers', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', working('Asking Genie'));
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    const c = el.querySelector('[data-genie="answer"]');
    expect(c?.querySelector('[data-genie-question]')?.textContent).toBe('What tables can I see?');
    expect(c?.querySelector('[data-answer] strong')?.textContent).toBe('Tables');
    expect(c?.querySelector('[data-answer]')?.textContent).not.toContain('**');
    expect([...(c?.querySelectorAll('.genie-keys li') ?? [])].map((li) => li.textContent)).toEqual([
      '8,800tables',
      '150catalogs',
    ]);
    expect(c?.querySelector('.genie-key-value')?.textContent).toBe('8,800');
    expect(c?.querySelector('button[data-action="open-in-bottle"]')?.textContent).toBe(
      OPEN_IN_BOTTLE_LABEL,
    );
    expect(c?.querySelector('button[data-action="dismiss"]')?.getAttribute('aria-label')).toBe(
      CLOSE_LABEL,
    );
    // The card never takes the keyboard from the user's app.
    expect(document.activeElement).toBe(document.body);
  });

  it('[#103] a single click on the card body opens Conversations, the same as a double-click', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    await mouse(el.querySelector('[data-genie]') as Element, 'click');
    expect(clicks()[0]?.payload).toEqual({ target: 'open_conversations', requestId: OP });
    expect(el.querySelector('[data-genie]')?.getAttribute('data-state')).toBe('dismissing');
  });

  // [LOW, round 2] "also fix if cheap": a press that moves before release is a drag, not a
  // click, and must never open Conversations. The old guard (checking `document.getSelection()`)
  // did nothing, because `.genie` is `user-select:none` and never has a selection to find.
  it('[#598] a press-drag-release on the card body never opens Conversations', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    const card = el.querySelector('[data-genie]') as Element;
    await act(async () => {
      card.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, clientX: 10, clientY: 10 }));
      card.dispatchEvent(new MouseEvent('click', { bubbles: true, clientX: 60, clientY: 10 }));
    });
    expect(clicks()).toEqual([]);
    // A real click (press and release at the same point) still opens it.
    await act(async () => {
      card.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, clientX: 10, clientY: 10 }));
      card.dispatchEvent(new MouseEvent('click', { bubbles: true, clientX: 11, clientY: 9 }));
    });
    expect(clicks()[0]?.payload).toEqual({ target: 'open_conversations', requestId: OP });
  });

  it('[#103] a click on [Open in Bottle] opens Conversations and does not bubble twice', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    const btn = el.querySelector('button[data-action="open-in-bottle"]') as HTMLElement;
    await act(async () => void btn.click());
    expect(clicks()[0]?.payload).toEqual({ target: 'open_conversations', requestId: OP });
  });

  it('[#103] a click on the close control fades the card at once and never opens Conversations', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    const close = el.querySelector('button[data-action="dismiss"]') as HTMLElement;
    await act(async () => void close.click());
    expect(clicks()).toEqual([]);
    expect(el.querySelector('[data-genie]')?.getAttribute('data-state')).toBe('dismissing');
    await advance(DISMISS_FADE_MS);
    expect(done()).toBe(1);
  });

  it('[#103] a long answer clamps to a short summary with Show more, then Show less', async () => {
    const el = await mount();
    const longMd = Array.from({ length: 20 }, (_, i) => `Paragraph number ${i}.`).join('\n\n');
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      ...answered,
      sections: [{ id: 'answer', md: longMd }],
    });
    const body = el.querySelector('[data-answer] .genie-answer-body') as HTMLElement;
    Object.defineProperty(body, 'scrollHeight', { value: ANSWER_CLAMP_PX * 3, configurable: true });
    Object.defineProperty(body, 'clientHeight', { value: ANSWER_CLAMP_PX, configurable: true });
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      ...answered,
      sections: [{ id: 'answer', md: `${longMd} ` }],
    });
    const toggle = () => el.querySelector('button.genie-toggle') as HTMLElement | null;
    expect(toggle()?.textContent).toBe(SHOW_MORE_LABEL);
    expect(body.getAttribute('data-expanded')).toBe('false');
    await act(async () => void toggle()?.click());
    expect(body.getAttribute('data-expanded')).toBe('true');
    expect(toggle()?.textContent).toBe(SHOW_LESS_LABEL);
    // Expanding or collapsing is not a click to open Conversations.
    expect(clicks()).toEqual([]);
    await act(async () => void toggle()?.click());
    expect(body.getAttribute('data-expanded')).toBe('false');
  });

  // [round 3, blocking] Show more, scroll to read the end, then Show less: the collapsed summary
  // must start at the top of the answer again, not wherever the user last scrolled to (which cut
  // the top line's glyphs instead of showing the heading).
  it('[#598] Show less resets the scroll position, so the collapsed summary starts at the top', async () => {
    const el = await mount();
    const longMd = Array.from({ length: 20 }, (_, i) => `Paragraph number ${i}.`).join('\n\n');
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      ...answered,
      sections: [{ id: 'answer', md: longMd }],
    });
    const body = el.querySelector('[data-answer] .genie-answer-body') as HTMLElement;
    Object.defineProperty(body, 'scrollHeight', { value: ANSWER_CLAMP_PX * 3, configurable: true });
    Object.defineProperty(body, 'clientHeight', { value: ANSWER_CLAMP_PX, configurable: true });
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      ...answered,
      sections: [{ id: 'answer', md: `${longMd} ` }],
    });
    const toggle = () => el.querySelector('button.genie-toggle') as HTMLElement | null;
    await act(async () => void toggle()?.click());
    expect(body.getAttribute('data-expanded')).toBe('true');
    // The user scrolls down to read the end of the answer.
    body.scrollTop = 151;
    expect(body.scrollTop).toBe(151);
    await act(async () => void toggle()?.click());
    expect(body.getAttribute('data-expanded')).toBe('false');
    expect(body.scrollTop).toBe(0);
  });

  // Review round 1, finding 2: one primary action leads ([Open in Bottle], the accent pill);
  // "Show more"/"Show less" is a quiet text control, never a second pill.
  it('[#598] Show more is a quiet control, not a second accent pill', async () => {
    const el = await mount();
    const longMd = Array.from({ length: 20 }, (_, i) => `Paragraph number ${i}.`).join('\n\n');
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      ...answered,
      sections: [{ id: 'answer', md: longMd }],
    });
    const body = el.querySelector('[data-answer] .genie-answer-body') as HTMLElement;
    Object.defineProperty(body, 'scrollHeight', { value: ANSWER_CLAMP_PX * 3, configurable: true });
    Object.defineProperty(body, 'clientHeight', { value: ANSWER_CLAMP_PX, configurable: true });
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      ...answered,
      sections: [{ id: 'answer', md: `${longMd} ` }],
    });
    const open = el.querySelector('button[data-action="open-in-bottle"]') as HTMLElement;
    const toggle = el.querySelector('button.genie-toggle') as HTMLElement;
    // [Open in Bottle] is the one accent-pill action; Show more carries no pill class at all.
    expect(open.classList.contains('popup-action')).toBe(true);
    expect(toggle.classList.contains('popup-action')).toBe(false);
  });

  // Review round 1, finding 1: when expanded, the footer ([Open in Bottle], Show less) and the
  // close control stay fixed outside the answer's own scroll area, so they never scroll away.
  it('[#598] the footer and the close control sit outside the answer scroll area', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    const scroller = el.querySelector('.genie-answer-body') as HTMLElement;
    const close = el.querySelector('button[data-action="dismiss"]') as HTMLElement;
    const open = el.querySelector('button[data-action="open-in-bottle"]') as HTMLElement;
    expect(close.closest('.genie-answer-body')).toBeNull();
    expect(open.closest('.genie-answer-body')).toBeNull();
    expect(scroller.querySelector('button[data-action="dismiss"]')).toBeNull();
    expect(scroller.querySelector('button[data-action="open-in-bottle"]')).toBeNull();
  });

  // Review round 2, finding 1: the card clipped its own footer when the answer had key numbers
  // (max-height 280 + overflow hidden, plus a fixed 180px expanded body: the parts added up to
  // more than 280). jsdom cannot lay out flex to catch the clip itself, so this checks the actual
  // rule, the same way `Popup.test.tsx` checks CSS it cannot render: the card is a flex column,
  // the body is the one part that may give up height, and everything else is `flex: none`.
  it('[#598] the card is a flex column so the footer and key numbers can never be squeezed out', async () => {
    const css = readFileSync(join(__dirname, 'genie-card.css'), 'utf8');
    expect(css).toMatch(
      /\.genie-answer,\s*\n\.genie-failure,\s*\n\.genie-message\s*\{[^}]*display:\s*flex;[^}]*flex-direction:\s*column;/,
    );
    expect(css).toMatch(/\.genie-body\s*\{[^}]*display:\s*flex;[^}]*min-height:\s*0;/);
    expect(css).toMatch(/\.genie-md\s*\{[^}]*flex:\s*1 1 auto;[^}]*min-height:\s*0;/);
    expect(css).toMatch(
      /\.genie-question,\n\.genie-keys,\n\.genie-failure-body,\n\.genie-foot\s*\{\s*\n\s*flex:\s*none;/,
    );
    // The flex safety net on the expanded body itself, alongside the measured, per-toggle cap.
    expect(css).toMatch(
      /\.genie-answer-body\[data-expanded='true'\]\s*\{[^}]*flex:\s*1 1 auto;[^}]*min-height:\s*0;/,
    );
  });

  it('[#573] an ignored answer fades after its show time and tells the brain, once', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    await advance(answerShowMs(ANSWER_MD) - 1);
    expect(el.querySelector('[data-genie]')?.getAttribute('data-state')).toBe('shown');
    await advance(1);
    expect(el.querySelector('[data-genie]')?.getAttribute('data-state')).toBe('fading');
    expect(done()).toBe(0);
    await advance(FADE_MS);
    expect(el.querySelector('[data-genie]')).toBeNull();
    expect(done()).toBe(1);
    await advance(60_000);
    expect(done()).toBe(1);
  });

  it('[#573] the pointer on the card holds it; after it leaves the card stays 2.5 s or more', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    await advance(answerShowMs(ANSWER_MD) - 500);
    const card = el.querySelector('[data-genie]') as Element;
    await mouse(card, 'mouseover');
    await advance(60_000);
    expect(card.getAttribute('data-state')).toBe('shown');
    await mouse(card, 'mouseout');
    await advance(AFTER_HOVER_MS - 1);
    expect(card.getAttribute('data-state')).toBe('shown');
    await advance(1 + FADE_MS);
    expect(done()).toBe(1);
  });

  it('[#573] the pointer on the MOG holds the card too (card:mog_hover)', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    await tauri.emitTo(`card-${ID}`, 'card:mog_hover', { on: true });
    await advance(60_000);
    expect(el.querySelector('[data-genie]')?.getAttribute('data-state')).toBe('shown');
    await tauri.emitTo(`card-${ID}`, 'card:mog_hover', { on: false });
    // The hold came at once, so all the show time is still left.
    await advance(answerShowMs(ANSWER_MD) - 1);
    expect(done()).toBe(0);
    await advance(1 + FADE_MS);
    expect(done()).toBe(1);
  });

  it('[#573] a double-click opens Conversations on this exchange and fades the card at once', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    await mouse(el.querySelector('[data-genie]') as Element, 'dblclick');
    expect(clicks()[0]?.payload).toEqual({ target: 'open_conversations', requestId: OP });
    // The quick fade has its own state, so its whole motion fits in its time.
    expect(el.querySelector('[data-genie]')?.getAttribute('data-state')).toBe('dismissing');
    await advance(DISMISS_FADE_MS);
    expect(done()).toBe(1);
  });

  it('[#573] a double-click on the MOG (card:dismiss) fades the answer, but never the working pill', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', working('Asking Genie'));
    await tauri.emitTo(`card-${ID}`, 'card:dismiss', {});
    await advance(1_000);
    expect(el.querySelector('[data-genie="working"]')).not.toBeNull();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    await tauri.emitTo(`card-${ID}`, 'card:dismiss', {});
    await advance(DISMISS_FADE_MS);
    expect(done()).toBe(1);
  });

  it('[GL-5] [W-07h] a workspace failure keeps one button (Try again) and adds a text link', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', workspaceFailed);
    const c = el.querySelector('[data-genie="failure"]');
    expect(c?.querySelector('[role="alert"]')?.textContent).toContain('Are you on the VPN?');
    const foot = c?.querySelector('footer') as Element;
    // One primary action: a single `popup-action` button. The link is a quiet text control.
    expect(foot.querySelectorAll('button.popup-action')).toHaveLength(1);
    const link = foot.querySelector(`[data-chip="workspace:${G}"]`) as HTMLElement;
    expect(link.textContent).toBe('Change workspace…');
    expect(link.classList.contains('popup-action')).toBe(false);
    await act(async () => link.click());
    expect(inputEmits()).toContainEqual({
      mogId: ID,
      kind: 'chip_click',
      payload: { chipId: `workspace:${G}` },
    });
    // It does not open the hub Conversations (no body click).
    expect(clicks()).toEqual([]);
  });

  it('[GL-5] a failure with no workspace chip shows no Change workspace link', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', failed);
    expect(el.querySelector('[data-genie="failure"]')?.textContent).not.toContain(
      'Change workspace',
    );
  });

  it('[GL-5] a failure shows the fixed text plainly with Try again, and stays 8 s', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', failed);
    const c = el.querySelector('[data-genie="failure"]');
    expect(c?.querySelector('[role="alert"]')?.textContent).toBe(
      `${FAILURE_TITLE}Genie could not answer this. Try again.`,
    );
    expect(c?.textContent).not.toContain('GENIE_FAILED');
    await act(async () => (c?.querySelector(`[data-chip="retry:${G}"]`) as HTMLElement).click());
    expect(inputEmits()).toContainEqual({
      mogId: ID,
      kind: 'chip_click',
      payload: { chipId: `retry:${G}` },
    });
    // A double-click on Try again sends it once and does not open the hub.
    const btn = c?.querySelector(`[data-chip="retry:${G}"]`) as Element;
    await act(
      async () => void btn.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 2 })),
    );
    await mouse(btn, 'dblclick');
    expect(inputEmits().filter((e) => (e as { kind: string }).kind === 'chip_click')).toHaveLength(
      1,
    );
    expect(clicks()).toEqual([]);
    await advance(FAILURE_SHOW_MS - 1);
    expect(c?.getAttribute('data-state')).toBe('shown');
    await advance(1 + FADE_MS);
    expect(done()).toBe(1);
  });
});

describe('Genie card: no flash (#573)', () => {
  beforeEach(() => tauri.reset());
  afterEach(async () => {
    await act(async () => roots.forEach((r) => r.unmount()));
    roots = [];
    tauri.label = 'hub';
  });

  it('the ⌥G echo of the question shows nothing before the status pill', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', { mogId: ID, blurbs: [ask], chips: [] });
    expect(el.querySelector('[data-popup]')).toBeNull();
    await tauri.emitTo(`card-${ID}`, 'card:update', working('Asking Genie'));
    expect(el.querySelector('[data-genie="working"]')).not.toBeNull();
  });

  it('a question with its Ask Genie chip still shows the pop-up', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      mogId: ID,
      blurbs: [ask],
      chips: [{ id: 'ask_genie:u1', label: 'Ask Genie' }],
    });
    expect(el.querySelector('[data-popup]')).not.toBeNull();
  });

  it('a faded answer that the brain cleared shows nothing when the card loads again', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      mogId: ID,
      blurbs: [{ id: G, text: '', pinned: false, completed: true }, ask],
      chips: [],
    });
    expect(el.querySelector('[data-genie]')).toBeNull();
  });
});

describe('Genie card: the window fit (#573)', () => {
  beforeEach(() => tauri.reset());
  afterEach(async () => {
    await act(async () => roots.forEach((r) => r.unmount()));
    roots = [];
    tauri.label = 'hub';
  });

  it('a gone card asks for one line, so a later show never gets the old tall window', async () => {
    vi.useFakeTimers();
    const sizes: number[] = [];
    const { GenieCard } = await import('./GenieCard');
    tauri.label = `card-${ID}`;
    const r = await render(
      <section>
        <GenieCard
          mogId={ID}
          card={answered as unknown as CardUpdate}
          onSize={(px) => void sizes.push(px)}
        />
      </section>,
    );
    roots.push(r.root);
    await act(async () => void vi.advanceTimersByTime(answerShowMs(ANSWER_MD) + FADE_MS));
    vi.useRealTimers();
    expect(r.el.querySelector('[data-genie]')).toBeNull();
    expect(sizes.at(-1)).toBe(0);
  });

  // Review round 2, finding 2: growing the window only after `transitionend` left the footer
  // below the window edge for the ~200 ms the body's `max-height` animated, then the window
  // snapped open. On Show more the window must grow to its target size *first* ("the window
  // fits the final size first", the pattern this file already uses for the pill-to-card grow
  // motion), and only then does the body open. Collapsing keeps the round-1 order: the body
  // closes first, and the window shrinks once that settles (`transitionend`).
  it('[#598] Show more grows the window before the body opens; Show less shrinks it after the body closes', async () => {
    const sizes: number[] = [];
    // A controllable `onSize`, so the test can see the window mid-resize instead of guessing at
    // `act`'s microtask timing.
    let resolveSize: (() => void) | undefined;
    const pushSize = (px: number) => {
      sizes.push(px);
      return new Promise<void>((resolve) => {
        resolveSize = resolve;
      });
    };
    const longMd = Array.from({ length: 20 }, (_, i) => `Paragraph number ${i}.`).join('\n\n');
    const { GenieCard } = await import('./GenieCard');
    tauri.label = `card-${ID}`;
    const card = { ...answered, sections: [{ id: 'answer', md: longMd }] } as unknown as CardUpdate;
    const r = await render(
      <section>
        <GenieCard mogId={ID} card={card} onSize={pushSize} />
      </section>,
    );
    roots.push(r.root);
    const body = r.el.querySelector('.genie-answer-body') as HTMLElement;
    Object.defineProperty(body, 'scrollHeight', { value: ANSWER_CLAMP_PX * 3, configurable: true });
    Object.defineProperty(body, 'clientHeight', { value: ANSWER_CLAMP_PX, configurable: true });
    await act(async () => void resolveSize?.());
    await act(
      async () =>
        void r.root.render(
          <section>
            <GenieCard
              mogId={ID}
              card={
                {
                  ...answered,
                  sections: [{ id: 'answer', md: `${longMd} ` }],
                } as unknown as CardUpdate
              }
              onSize={pushSize}
            />
          </section>,
        ),
    );
    await act(async () => void resolveSize?.());

    // Show more: the window is already asked to grow, but the body stays closed until that
    // resize resolves.
    const before = sizes.length;
    const toggle = () => r.el.querySelector('button.genie-toggle') as HTMLElement;
    await act(async () => void toggle().click());
    expect(sizes.length).toBe(before + 1);
    expect(body.getAttribute('data-expanded')).toBe('false');
    await act(async () => void resolveSize?.());
    expect(body.getAttribute('data-expanded')).toBe('true');
    expect(toggle().textContent).toBe(SHOW_LESS_LABEL);

    // Show less: the body closes at once (no grow call yet); the window only shrinks once the
    // close transition settles.
    const beforeCollapse = sizes.length;
    await act(async () => void toggle().click());
    expect(body.getAttribute('data-expanded')).toBe('false');
    expect(sizes.length).toBe(beforeCollapse);
    await act(async () => {
      const ev = new Event('transitionend');
      Object.defineProperty(ev, 'propertyName', { value: 'max-height' });
      body.dispatchEvent(ev);
    });
    expect(sizes.length).toBe(beforeCollapse + 1);
  });
});

describe('Genie card links (#523)', () => {
  const WS = 'https://acme-prod.cloud.databricks.test';
  const DEEP = `${WS}/one/chat/threads/c1?o=1234&utm_source=genie_chat_mcp`;
  const withLinks = (md: string, footer: Record<string, string> = {}) => ({
    ...answered,
    sections: [
      { id: 'answer', md },
      { id: 'footer', deepLink: DEEP, workspaceHost: WS, ...footer },
      { id: 'dataFlow', text: 'x' },
    ],
  });
  beforeEach(() => {
    tauri.reset();
    vi.useFakeTimers();
  });
  afterEach(async () => {
    await act(async () => roots.forEach((r) => r.unmount()));
    roots = [];
    vi.useRealTimers();
    tauri.label = 'hub';
  });
  const linkClicks = () => clicks().filter((c) => c.payload.target === 'link_open');

  describe('the pill shows the question and the link', () => {
    const LONG =
      'Which customers in the northern region renewed their annual contract after the second price change last quarter, and what share of them moved to the new plan?';
    const pillWith = (text: string, q: string, sections?: unknown[]) => ({
      mogId: ID,
      blurbs: [
        { id: G, text, pinned: false, completed: false },
        { id: 'ask:u1', text: `"${q}"`, pinned: false, completed: true },
      ],
      chips: [],
      ...(sections ? { sections } : {}),
    });
    const footer = { id: 'footer', deepLink: DEEP, workspaceHost: WS };

    it('shows the whole long question, wrapped, with no ellipsis and no truncation', async () => {
      const el = await mount();
      await tauri.emitTo(`card-${ID}`, 'card:update', pillWith('Running the query', LONG));
      const q = el.querySelector('[data-genie="working"] [data-genie-pill-question]');
      // The whole question is in the DOM, not cut, with no truncation attribute or class.
      expect(q?.textContent).toBe(LONG);
      expect(q?.getAttribute('title')).toBeNull();
    });

    it('shows no link before the answer is ready, and none without a deep link', async () => {
      const el = await mount();
      await tauri.emitTo(
        `card-${ID}`,
        'card:update',
        pillWith('Running the query', 'Q?', [footer]),
      );
      expect(
        el.querySelector('[data-genie="working"] [data-action="open-in-databricks"]'),
      ).toBeNull();
      await tauri.emitTo(`card-${ID}`, 'card:update', pillWith(ANSWER_READY, 'Q?'));
      expect(
        el.querySelector('[data-genie="working"] [data-action="open-in-databricks"]'),
      ).toBeNull();
      await tauri.emitTo(
        `card-${ID}`,
        'card:update',
        pillWith(ANSWER_READY, 'Q?', [{ id: 'footer', deepLink: '', workspaceHost: WS }]),
      );
      expect(el.querySelector('[data-action="open-in-databricks"]')).toBeNull();
    });

    it('shows a link on the ready pill and sends the same link_open click', async () => {
      const el = await mount();
      await tauri.emitTo(`card-${ID}`, 'card:update', pillWith(ANSWER_READY, 'Q?', [footer]));
      const a = el.querySelector(
        '[data-genie="working"] a[data-action="open-in-databricks"]',
      ) as HTMLAnchorElement;
      expect(a.textContent).toBe('Open in Databricks');
      expect(a.getAttribute('href')).toBe(DEEP);
      expect(a.getAttribute('aria-label')).toBe('Open in Databricks (opens in browser)');
      const ev = new MouseEvent('click', { bubbles: true, cancelable: true, detail: 1 });
      await act(async () => void a.dispatchEvent(ev));
      expect(ev.defaultPrevented).toBe(true);
      expect(linkClicks()).toHaveLength(1);
      expect(linkClicks()[0]?.payload).toEqual({ target: 'link_open', url: DEEP });
      expect(clicks().some((c) => c.payload.target === 'open_conversations')).toBe(false);
    });

    it('reserves the link row before ready (hidden, not focusable) and the link replaces it', async () => {
      const el = await mount();
      await tauri.emitTo(
        `card-${ID}`,
        'card:update',
        pillWith('Running the query', 'Q?', [footer]),
      );
      const ph = el.querySelector('[data-genie="working"] [data-pill-link-placeholder]');
      expect(ph).not.toBeNull();
      expect(ph?.getAttribute('aria-hidden')).toBe('true');
      expect(ph?.tagName).not.toBe('A');
      expect(ph?.closest('.genie-pill-linkrow')).not.toBeNull();
      expect(el.querySelector('a[data-action="open-in-databricks"]')).toBeNull();
      await tauri.emitTo(`card-${ID}`, 'card:update', pillWith(ANSWER_READY, 'Q?', [footer]));
      expect(el.querySelector('[data-pill-link-placeholder]')).toBeNull();
      const row = el.querySelector('.genie-pill-linkrow');
      expect(row?.querySelector('a[data-action="open-in-databricks"]')).not.toBeNull();
      // The row stays: the ready moment adds no row, so the pill keeps its size.
      expect(el.querySelectorAll('.genie-pill-linkrow')).toHaveLength(1);
    });

    it('keeps the link outside the scroller for a question longer than the cap', async () => {
      const el = await mount();
      const huge = `${LONG} `.repeat(12).trim();
      await tauri.emitTo(`card-${ID}`, 'card:update', pillWith(ANSWER_READY, huge, [footer]));
      const q = el.querySelector('[data-genie-pill-question]') as HTMLElement;
      expect(q.textContent).toBe(huge);
      const a = el.querySelector('a[data-action="open-in-databricks"]');
      expect(a).not.toBeNull();
      expect(q.contains(a)).toBe(false);
      expect(q.contains(el.querySelector('[data-genie-stage]'))).toBe(false);
    });

    it('marks the question as overflowing only when it does', async () => {
      const el = await mount();
      const set = (scroll: number) => {
        Object.defineProperty(HTMLElement.prototype, 'scrollHeight', {
          configurable: true,
          get: () => scroll,
        });
      };
      try {
        set(100);
        await tauri.emitTo(`card-${ID}`, 'card:update', pillWith('Running', 'Q?'));
        expect(el.querySelector('[data-genie-pill-question]')?.hasAttribute('data-overflow')).toBe(
          true,
        );
        set(0);
        await tauri.emitTo(`card-${ID}`, 'card:update', pillWith('Running', 'Q2?'));
        expect(el.querySelector('[data-genie-pill-question]')?.hasAttribute('data-overflow')).toBe(
          false,
        );
      } finally {
        delete (HTMLElement.prototype as { scrollHeight?: number }).scrollHeight;
      }
    });

    it('takes no focus when the ready pill renders', async () => {
      const el = await mount();
      const focus = vi.spyOn(HTMLElement.prototype, 'focus');
      try {
        await tauri.emitTo(`card-${ID}`, 'card:update', pillWith('Running', 'Q?', [footer]));
        await tauri.emitTo(`card-${ID}`, 'card:update', pillWith(ANSWER_READY, 'Q?', [footer]));
        expect(el.querySelector('a[data-action="open-in-databricks"]')).not.toBeNull();
        expect(focus).not.toHaveBeenCalled();
      } finally {
        focus.mockRestore();
      }
    });

    it('has a close control on the working pill that hides the card and cancels nothing', async () => {
      const el = await mount();
      await tauri.emitTo(`card-${ID}`, 'card:update', pillWith('Running the query', 'Q?'));
      const close = el.querySelector(
        '[data-genie="working"] button[data-action="dismiss"]',
      ) as HTMLButtonElement;
      expect(close.getAttribute('aria-label')).toBe(CLOSE_LABEL);
      expect(close.tabIndex).toBe(0);
      expect(document.activeElement).not.toBe(close);
      await act(async () => void close.click());
      expect(clicks()).toEqual([]);
      await advance(DISMISS_FADE_MS);
      // Only the card is done, for this request; no cancel, no Conversations.
      expect(clicks().map((c) => c.payload)).toEqual([{ target: 'card_done', requestId: OP }]);
    });

    it('has the same close control on the ready pill', async () => {
      const el = await mount();
      await tauri.emitTo(`card-${ID}`, 'card:update', pillWith(ANSWER_READY, 'Q?', [footer]));
      const close = el.querySelector(
        '[data-genie="working"] button[data-action="dismiss"]',
      ) as HTMLButtonElement;
      await act(async () => void close.click());
      await advance(DISMISS_FADE_MS);
      expect(done()).toBe(1);
    });

    it('shows the answer when it lands after the user closed the working pill', async () => {
      const el = await mount();
      await tauri.emitTo(`card-${ID}`, 'card:update', pillWith('Running the query', 'Q?'));
      const close = el.querySelector('button[data-action="dismiss"]') as HTMLButtonElement;
      await act(async () => void close.click());
      await advance(DISMISS_FADE_MS);
      expect(el.querySelector('[data-genie]')).toBeNull();
      await tauri.emitTo(`card-${ID}`, 'card:update', answered);
      expect(el.querySelector('[data-genie="answer"]')).not.toBeNull();
    });
  });

  it('[#523] shows Open in Databricks for a trusted deep_link and sends one link_open click', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', withLinks('Hello.'));
    const a = el.querySelector('a[data-action="open-in-databricks"]') as HTMLAnchorElement;
    expect(a.textContent).toBe('Open in Databricks');
    expect(a.getAttribute('title')).toBe(DEEP);
    expect(a.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
    const ev = new MouseEvent('click', { bubbles: true, cancelable: true, detail: 1 });
    await act(async () => void a.dispatchEvent(ev));
    expect(ev.defaultPrevented).toBe(true);
    expect(linkClicks()).toHaveLength(1);
    expect(linkClicks()[0]?.payload).toEqual({ target: 'link_open', url: DEEP });
    // A click on the link never opens the hub or fades the card.
    expect(clicks().some((c) => c.payload.target === 'open_conversations')).toBe(false);
    expect(done()).toBe(0);
  });

  it.each([
    ['no deep_link', { deepLink: '' }],
    ['a deep_link on another host', { deepLink: 'https://evil.test/one/chat/threads/c1' }],
    ['a deep_link that spoofs the host', { deepLink: `${WS}.evil.test/one/chat/threads/c1` }],
    ['an http deep_link', { deepLink: DEEP.replace('https:', 'http:') }],
    ['no known workspace host', { workspaceHost: '' }],
  ])('[#523] shows no Open in Databricks with %s', async (_n, footer) => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', withLinks('Hello.', footer));
    expect(el.querySelector('[data-action="open-in-databricks"]')).toBeNull();
  });

  it('[#523] shows no Open in Databricks on a failure card', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', failed);
    expect(el.querySelector('[data-action="open-in-databricks"]')).toBeNull();
  });

  it('[#523] a trusted link in the answer is clickable and sends link_open; another host is text', async () => {
    const el = await mount();
    await tauri.emitTo(
      `card-${ID}`,
      'card:update',
      withLinks(`[the chat](${WS}/sql/dashboards/9f2) and [docs](https://evil.test/d)`),
    );
    const links = [...el.querySelectorAll('.genie-answer-body a')] as HTMLAnchorElement[];
    expect(links.map((a) => a.textContent)).toEqual(['the chat']);
    expect(el.querySelector('.genie-answer-body .md-link--blocked')?.textContent).toContain(
      'evil.test',
    );
    await act(
      async () =>
        void links[0]?.dispatchEvent(
          new MouseEvent('click', { bubbles: true, cancelable: true, detail: 1 }),
        ),
    );
    expect(linkClicks().map((c) => c.payload)).toEqual([
      { target: 'link_open', url: `${WS}/sql/dashboards/9f2` },
    ]);
    expect(clicks().some((c) => c.payload.target === 'open_conversations')).toBe(false);
  });

  it('[#523] with no workspace host in the footer every link in the answer is text', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      ...answered,
      sections: [{ id: 'answer', md: `[x](${WS}/a) ${WS}/b` }, { id: 'footer' }],
    });
    expect(el.querySelector('.genie-answer-body a')).toBeNull();
  });

  it('[#523] a table in the answer renders with readable numbers', async () => {
    const el = await mount();
    await tauri.emitTo(
      `card-${ID}`,
      'card:update',
      withLinks('| a | b |\n| - | - |\n| x | 1220042.3509332137 |'),
    );
    expect(el.querySelector('.genie-answer-body table')).not.toBeNull();
    expect(el.querySelector('.genie-answer-body tbody td:last-child')?.textContent).toBe(
      new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(1220042.3509332137),
    );
  });

  describe('UX slice 1: ready pill and stacked pop-up', () => {
    const dictation = { id: 'd1', text: 'Quick update before the board deck', completed: true };
    const stacked = (text: string) => ({
      mogId: ID,
      blurbs: [dictation, { id: G, text, pinned: false, completed: false }, ask],
      chips: [{ id: 'ask_genie:d1', label: 'Ask Genie' }],
    });

    it('[AC-1.2] "Answer ready" shows as the ready pill with no timer, and never takes focus', async () => {
      const el = await mount();
      await tauri.emitTo(`card-${ID}`, 'card:update', working('Asking Genie'));
      await advance(5_000);
      expect(el.querySelector('[data-genie-timer]')).not.toBeNull();
      await tauri.emitTo(`card-${ID}`, 'mog:target', {
        mogId: ID,
        name: 'Sales',
        target: true,
        toast: false,
        colorVariant: 'teal',
      });
      await tauri.emitTo(`card-${ID}`, 'card:update', working(ANSWER_READY));
      const pill = el.querySelector('[data-genie="working"]');
      expect(pill?.hasAttribute('data-ready')).toBe(true);
      expect(pill?.querySelector('[data-genie-stage]')?.textContent).toBe(ANSWER_READY);
      expect(pill?.querySelector('[data-genie-live]')?.textContent).toBe('Sales: Answer ready');
      expect(pill?.querySelector('.genie-ready-dot')).not.toBeNull();
      expect(pill?.querySelector('[data-genie-timer]')).toBeNull();
      // The ready pill never fades and takes no focus (AC-1.14).
      await advance(60_000);
      expect(el.querySelector('[data-genie="working"]')).not.toBeNull();
      expect(done()).toBe(0);
      expect(document.activeElement).toBe(document.body);
    });

    it('[AC-1.2] the ready pill keeps the width of the working pill as an inline min-width', async () => {
      const real = HTMLElement.prototype.getBoundingClientRect;
      HTMLElement.prototype.getBoundingClientRect = function (this: HTMLElement) {
        const w = this.classList.contains('genie-working') ? 231 : 0;
        return { ...real.call(this), width: w } as DOMRect;
      };
      try {
        const el = await mount();
        await tauri.emitTo(`card-${ID}`, 'card:update', working('Running the query'));
        await advance(8_000);
        const pill = el.querySelector('[data-genie="working"]') as HTMLElement;
        expect(pill.style.minWidth).toBe('');
        await tauri.emitTo(`card-${ID}`, 'card:update', working(ANSWER_READY));
        expect((el.querySelector('[data-genie="working"]') as HTMLElement).style.minWidth).toBe(
          '231px',
        );
      } finally {
        HTMLElement.prototype.getBoundingClientRect = real;
      }
    });

    it('[AC-1.12] a dictation pop-up shows under the pending pill and does not replace it', async () => {
      const el = await mount();
      await tauri.emitTo(`card-${ID}`, 'card:update', stacked('Running the query'));
      const pill = el.querySelector('[data-genie="working"]');
      const popup = el.querySelector('[data-popup]');
      expect(pill?.querySelector('[data-genie-stage]')?.textContent).toBe('Running the query');
      expect(popup?.textContent).toContain('Quick update before the board deck');
      // The pill comes first, the pop-up under it, in the one card section.
      expect(pill?.parentElement).toBe(popup?.parentElement);
      expect(pill?.compareDocumentPosition(popup as Node)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
      expect(document.activeElement).toBe(document.body);
    });

    it('[AC-1.12] the pill keeps its timer when the pop-up comes (no remount)', async () => {
      const el = await mount();
      await tauri.emitTo(`card-${ID}`, 'card:update', working('Asking Genie'));
      await advance(4_000);
      expect(el.querySelector('[data-genie-timer]')?.textContent).toBe('0:04');
      await tauri.emitTo(`card-${ID}`, 'card:update', stacked('Asking Genie'));
      await advance(1_000);
      expect(el.querySelector('[data-genie-timer]')?.textContent).toBe('0:05');
      expect(el.querySelector('[data-popup]')).not.toBeNull();
    });

    it('[AC-1.12] a pop-up that fades leaves the pill and does not hide the card window', async () => {
      const el = await mount();
      await tauri.emitTo(`card-${ID}`, 'card:update', {
        ...stacked('Running the query'),
        chips: [],
      });
      expect(el.querySelector('[data-popup]')).not.toBeNull();
      await advance(FADE_AFTER_MS);
      await advance(FADE_OUT_MS);
      expect(el.querySelector('[data-popup]')).toBeNull();
      expect(el.querySelector('[data-genie="working"]')).not.toBeNull();
      // The pop-up sends no `cancel`: the brain would hide the whole card.
      expect(inputEmits().filter((e) => (e as { kind: string }).kind === 'cancel')).toEqual([]);
    });

    it('with no pending Genie request the pop-up alone still sends cancel when it fades', async () => {
      const el = await mount();
      await tauri.emitTo(`card-${ID}`, 'card:update', {
        mogId: ID,
        blurbs: [dictation],
        chips: [],
      });
      await advance(FADE_AFTER_MS);
      await advance(FADE_OUT_MS);
      expect(el.querySelector('[data-popup]')).toBeNull();
      expect(inputEmits().filter((e) => (e as { kind: string }).kind === 'cancel')).toHaveLength(1);
    });
  });
});
