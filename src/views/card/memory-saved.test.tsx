// @vitest-environment jsdom
/**
 * Memory slice 1 (ADR 0113), S1 and S2: "Remember this" on the Genie card and the "Saved to
 * today's note · Undo" line on the card and the dictation pop-up. All values are plainly fake.
 */
import { act } from 'react';
import type { Root } from 'react-dom/client';
import { inputEmits, render, tauri } from '../test-tauri';
import { CardView } from './CardView';
import { REMEMBER_WAIT_MS } from './GenieCard';
import { FAILED_LINE, QUARANTINE_LINE, SAVED_LINE, SECRET_LINE, UNDONE_LINE } from './memory-saved';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const ID = '00000000-0000-4000-8000-000000000001';
const OP = '00000000-0000-4000-8000-00000000c573';
const ITEM = '51111111-2222-4333-8444-000000000001';
const ask = { id: 'ask:u1', text: '"What tables can I see?"', pinned: false, completed: true };
const answered = {
  mogId: ID,
  blurbs: [
    { id: `genie:${OP}`, text: 'You can see about 8,800 tables.', pinned: false, completed: true },
    ask,
  ],
  chips: [],
  sections: [{ id: 'answer', md: 'You can see about **8,800 tables**.' }, { id: 'footer' }],
};
const dictated = {
  mogId: ID,
  blurbs: [{ id: 'u9', text: 'remember this: the fake mart is at six', completed: true }],
  chips: [],
  historyCount: 0,
};

let roots: Root[] = [];
async function mount() {
  tauri.label = `card-${ID}`;
  const r = await render(<CardView mogId={ID} />);
  roots.push(r.root);
  return r.el;
}
const saved = (p: object) =>
  tauri.emitTo(`card-${ID}`, 'memory:saved', { mogId: ID, undoMs: 8000, ...p });
const clicks = () =>
  inputEmits().filter((e) => (e as { kind: string }).kind === 'click') as {
    payload: Record<string, unknown>;
  }[];
const line = (el: HTMLElement) => el.querySelector('[data-memory-saved]') as HTMLElement | null;
const advance = (ms: number) => act(async () => void vi.advanceTimersByTime(ms));

describe('memory slice 1: card and pop-up', () => {
  beforeEach(() => {
    tauri.reset();
    vi.useFakeTimers();
  });
  afterEach(async () => {
    await act(async () => roots.forEach((r) => r.unmount()));
    roots = [];
    vi.useRealTimers();
  });

  it('S1 the answer card has a Remember this button that sends memory.remember', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    const btn = el.querySelector('[data-action="remember"]') as HTMLButtonElement;
    expect(btn.textContent).toBe('Remember this');
    expect(btn.getAttribute('aria-label')).toBeTruthy();
    await act(async () => void btn.click());
    expect(clicks().filter((c) => c.payload['action'] === 'memory.remember')).toEqual([
      { mogId: ID, kind: 'click', payload: { action: 'memory.remember', requestId: OP } },
    ]);
    // One click only, while the brain works.
    expect(btn.disabled).toBe(true);
    await act(async () => void btn.click());
    expect(clicks().filter((c) => c.payload['action'] === 'memory.remember')).toHaveLength(1);
    // The click does not open Conversations.
    expect(clicks().some((c) => c.payload['target'] === 'open_conversations')).toBe(false);
  });

  it('S1 saved shows the line with Undo, and Undo sends memory.undo', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    await saved({ outcome: 'saved', itemId: ITEM });
    expect(line(el)?.textContent).toBe(`${SAVED_LINE}·Undo`);
    expect(el.querySelector('[data-action="remember"]')).toBeNull();
    const undo = el.querySelector('[data-action="memory-undo"]') as HTMLButtonElement;
    expect(undo.getAttribute('aria-label')).toContain('Undo');
    await act(async () => void undo.click());
    expect(clicks().filter((c) => c.payload['action'] === 'memory.undo')).toEqual([
      { mogId: ID, kind: 'click', payload: { action: 'memory.undo', itemId: ITEM } },
    ]);
    expect(line(el)?.textContent).toBe(UNDONE_LINE);
    await advance(3000);
    expect(line(el)).toBeNull();
  });

  it('S1 the line stays for undoMs and keeps the card open', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    await advance(2000);
    await saved({ outcome: 'saved', itemId: ITEM });
    await advance(7900);
    expect(line(el)).not.toBeNull();
    expect(el.querySelector('[data-genie="answer"]')?.getAttribute('data-state')).toBe('shown');
    await advance(200);
    expect(line(el)).toBeNull();
  });

  it('S1 shows the refusal, the quarantine note and the failure line', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    await saved({ outcome: 'secret_refused' });
    expect(line(el)?.textContent).toBe(SECRET_LINE);
    expect(line(el)?.querySelector('[data-action="memory-undo"]')).toBeNull();
    await saved({ outcome: 'quarantined', itemId: ITEM });
    expect(line(el)?.textContent).toBe(QUARANTINE_LINE);
    expect(line(el)?.querySelector('[data-action="memory-undo"]')).toBeNull();
    await saved({ outcome: 'failed' });
    expect(line(el)?.textContent).toBe(FAILED_LINE);
    // After a failure the user can try again.
    expect(el.querySelector('[data-action="remember"]')).not.toBeNull();
  });

  it('S1 ignores a memory:saved of another MOG and a bad payload', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    await tauri.emitTo(`card-${ID}`, 'memory:saved', {
      mogId: 'other',
      outcome: 'saved',
      itemId: ITEM,
      undoMs: 8000,
    });
    await tauri.emitTo(`card-${ID}`, 'memory:saved', { mogId: ID, outcome: 'nope' });
    expect(line(el)).toBeNull();
  });

  it('S2 the dictation pop-up shows Saved and Undo for a voice remember', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', dictated);
    await saved({ outcome: 'saved', itemId: ITEM });
    expect(el.querySelector('[data-popup] [data-memory-saved]')?.textContent).toBe(
      `${SAVED_LINE}·Undo`,
    );
    // The pop-up does not fade while Undo shows.
    await advance(6000);
    expect(el.querySelector('[data-popup]')).not.toBeNull();
    await act(
      async () => void (el.querySelector('[data-action="memory-undo"]') as HTMLElement).click(),
    );
    expect(clicks().filter((c) => c.payload['action'] === 'memory.undo')).toHaveLength(1);
  });

  it('S2 the pop-up shows the secret refusal', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', dictated);
    await saved({ outcome: 'secret_refused' });
    expect(el.querySelector('[data-popup] [data-memory-saved]')?.textContent).toBe(SECRET_LINE);
  });

  it('S2 a voice save with no blurb and no earlier card:update shows the line', async () => {
    const el = await mount();
    await saved({ outcome: 'saved', itemId: ITEM });
    expect(el.querySelector('[data-popup] [data-memory-saved]')?.textContent).toBe(
      `${SAVED_LINE}·Undo`,
    );
    await act(
      async () => void (el.querySelector('[data-action="memory-undo"]') as HTMLElement).click(),
    );
    expect(clicks().filter((c) => c.payload['action'] === 'memory.undo')).toHaveLength(1);
  });

  it('S2 the line survives a new blurb (the pop-up remounts)', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', dictated);
    await saved({ outcome: 'saved', itemId: ITEM });
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      ...dictated,
      blurbs: [{ id: 'u10', text: 'Another line', completed: true }],
    });
    expect(el.querySelector('[data-popup] [data-memory-saved]')).not.toBeNull();
  });

  it('S1 keeps a quiet In today’s note label after a save, and one live region stays mounted', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    expect(el.querySelector('.memory-saved-live')?.getAttribute('role')).toBe('status');
    await saved({ outcome: 'saved', itemId: ITEM });
    expect(el.querySelector('[data-action="remember"]')).toBeNull();
    expect(el.querySelector('[data-remembered]')?.textContent).toBe('In today’s note');
    await advance(8100);
    expect(line(el)).toBeNull();
    expect(el.querySelector('[data-remembered]')).not.toBeNull();
    expect(el.querySelector('.memory-saved-live')).not.toBeNull();
  });

  it('S1 a click with no answer frees the button after a wait', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    // The pointer holds the card open while the brain stays silent.
    await act(async () =>
      el
        .querySelector('.genie')
        ?.dispatchEvent(
          new MouseEvent('mouseover', { bubbles: true, relatedTarget: document.body }),
        ),
    );
    const btn = el.querySelector('[data-action="remember"]') as HTMLButtonElement;
    await act(async () => void btn.click());
    expect(btn.disabled).toBe(true);
    await advance(REMEMBER_WAIT_MS + 100);
    expect(btn.disabled).toBe(false);
  });

  it('memory.remember carries the exchange op id of this card as requestId', async () => {
    const el = await mount();
    const other = '00000000-0000-4000-8000-0000000000aa';
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      ...answered,
      blurbs: [{ ...answered.blurbs[0], id: `genie:${other}` }, ask],
    });
    await act(
      async () => void (el.querySelector('[data-action="remember"]') as HTMLElement).click(),
    );
    const sent = clicks().filter((c) => c.payload['action'] === 'memory.remember');
    expect(sent).toHaveLength(1);
    expect(sent[0]?.payload['requestId']).toBe(other);
  });

  it('memory.undo carries the UUID itemId from memory:saved, and a save with no itemId has no Undo', async () => {
    const el = await mount();
    await tauri.emitTo(`card-${ID}`, 'card:update', answered);
    await saved({ outcome: 'saved' });
    expect(el.querySelector('[data-action="memory-undo"]')).toBeNull();
    await saved({ outcome: 'saved', itemId: ITEM });
    await act(
      async () => void (el.querySelector('[data-action="memory-undo"]') as HTMLElement).click(),
    );
    const sent = clicks().filter((c) => c.payload['action'] === 'memory.undo');
    expect(sent).toHaveLength(1);
    expect(sent[0]?.payload['itemId']).toBe(ITEM);
    expect(sent[0]?.payload['itemId']).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
    );
  });
});
