// @vitest-environment jsdom
/** #573: the Ask Genie status pill and the transient answer card beside the MOG. */
import { act } from 'react';
import type { Root } from 'react-dom/client';
import { AFTER_HOVER_MS, FAILURE_SHOW_MS, answerShowMs } from '../../shared/genie-status';
import { inputEmits, render, tauri } from '../test-tauri';
import { CardView } from './CardView';
import { DISMISS_FADE_MS, FADE_MS, FAILURE_TITLE, FOOTER_HINT } from './GenieCard';

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
    expect(c?.textContent).toContain(FOOTER_HINT);
    // The card never takes the keyboard from the user's app.
    expect(document.activeElement).toBe(document.body);
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
        <GenieCard mogId={ID} card={answered} onSize={(px) => void sizes.push(px)} />
      </section>,
    );
    roots.push(r.root);
    await act(async () => void vi.advanceTimersByTime(answerShowMs(ANSWER_MD) + FADE_MS));
    vi.useRealTimers();
    expect(r.el.querySelector('[data-genie]')).toBeNull();
    expect(sizes.at(-1)).toBe(0);
  });
});
