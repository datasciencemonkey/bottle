// @vitest-environment jsdom
/**
 * The hub thread reply composer (genie-followup, #145, #103): a text input and Send at the
 * bottom of a thread's detail. Sending appends the question to that thread's Genie conversation
 * (`thread.send`); a live status line (asking, then stage, then timer) tracks it until the
 * answer lands in the thread's own list. The hub window's focus on this thread is announced to
 * the brain (`thread.active` / `thread.inactive`), so a ⌥G press knows to follow up in it.
 */
import { act } from 'react';
import type { Root } from 'react-dom/client';
import { render, tauri, viewEmits } from '../test-tauri';
import { ThreadComposer } from './ThreadComposer';
import { HUB_THREAD_STATUS_EVENT } from '../../shared/hub-conversations';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const THREAD = { conversationId: 'conv-1', exchangeId: 'req-1', profile: 'fake-west' };

const field = (el: HTMLElement) => el.querySelector('textarea') as HTMLTextAreaElement;
const send = (el: HTMLElement) => el.querySelector('button[type="submit"]') as HTMLButtonElement;
const actions = () =>
  viewEmits().map((e) => (e as { payload?: unknown }).payload) as Record<string, unknown>[];
const sendActions = () => actions().filter((a) => a['action'] === 'thread.send');
const activeActions = () =>
  actions().filter((a) => a['action'] === 'thread.active' || a['action'] === 'thread.inactive');

/** A React-controlled `<textarea>` ignores a plain `.value =` write; it must go through the
 * native setter so React's value tracker sees it, exactly like `onChange` would. */
function type(el: HTMLTextAreaElement, text: string): void {
  const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  set?.call(el, text);
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

const roots: Root[] = [];
afterEach(() => {
  for (const r of roots.splice(0)) act(() => r.unmount());
});

async function mount(now: () => number = () => 1_000, thread = THREAD) {
  const r = await render(<ThreadComposer thread={thread} now={now} />);
  roots.push(r.root);
  return r;
}

describe('ThreadComposer (genie-followup, #145)', () => {
  beforeEach(() => tauri.reset());

  it('shows a text field, a Send button and the ⌥G hint', async () => {
    const { el } = await mount();
    expect(field(el)).not.toBeNull();
    expect(send(el).textContent).toBe('Send');
    expect(el.textContent).toContain('Or hold Right Option and tap G');
  });

  it('Send is disabled with no text, and never sends', async () => {
    const { el } = await mount();
    expect(send(el).disabled).toBe(true);
    await act(async () => void send(el).click());
    expect(sendActions()).toHaveLength(0);
  });

  it('sends the typed question to this thread, with its conversationId and profile, then clears the field', async () => {
    const { el } = await mount();
    await act(async () => {
      type(field(el), 'How many are there?');
    });
    expect(send(el).disabled).toBe(false);
    await act(async () => void send(el).click());
    expect(sendActions()).toEqual([
      {
        action: 'thread.send',
        conversationId: 'conv-1',
        profile: 'fake-west',
        question: 'How many are there?',
      },
    ]);
    expect(field(el).value).toBe('');
  });

  it('Enter sends; Shift+Enter does not', async () => {
    const { el } = await mount();
    await act(async () => {
      type(field(el), 'Follow-up?');
    });
    await act(async () => {
      field(el).dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Enter', shiftKey: true, bubbles: true }),
      );
    });
    expect(sendActions()).toHaveLength(0);
    await act(async () => {
      field(el).dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });
    expect(sendActions()).toHaveLength(1);
  });

  it('a blank or whitespace-only question never sends', async () => {
    const { el } = await mount();
    await act(async () => {
      type(field(el), '   ');
    });
    expect(send(el).disabled).toBe(true);
  });

  it('shows "Asking Genie", then a stage, then clears on the terminal status of this conversation', async () => {
    const { el } = await mount();
    await act(async () => {
      type(field(el), 'How many are there?');
    });
    await act(async () => void send(el).click());
    expect(el.textContent).toContain('Asking Genie');
    // Disabled while a request is in flight: no second question before the first answers.
    expect(field(el).disabled).toBe(true);

    await tauri.emitTo('hub', HUB_THREAD_STATUS_EVENT, {
      conversationId: 'conv-1',
      requestId: 'req-2',
      phase: 'working',
      text: 'Running the query',
    });
    expect(el.textContent).toContain('Running the query');

    await tauri.emitTo('hub', HUB_THREAD_STATUS_EVENT, {
      conversationId: 'conv-1',
      requestId: 'req-2',
      phase: 'answered',
      text: '',
    });
    expect(el.textContent).not.toContain('Running the query');
    expect(el.textContent).not.toContain('Asking Genie');
    expect(field(el).disabled).toBe(false);
  });

  it('ignores a status event of another conversation', async () => {
    const { el } = await mount();
    await act(async () => {
      type(field(el), 'How many are there?');
    });
    await act(async () => void send(el).click());
    await tauri.emitTo('hub', HUB_THREAD_STATUS_EVENT, {
      conversationId: 'conv-other',
      requestId: 'req-x',
      phase: 'working',
      text: 'Reading the results',
    });
    expect(el.textContent).not.toContain('Reading the results');
    expect(el.textContent).toContain('Asking Genie');
  });

  it("announces thread.active on mount, with this thread's conversationId and profile", async () => {
    await mount();
    expect(activeActions()).toEqual([
      { action: 'thread.active', conversationId: 'conv-1', profile: 'fake-west' },
    ]);
  });

  it('announces thread.inactive on blur, thread.active again on focus', async () => {
    await mount();
    await act(async () => void window.dispatchEvent(new Event('blur')));
    await act(async () => void window.dispatchEvent(new Event('focus')));
    expect(activeActions()).toEqual([
      { action: 'thread.active', conversationId: 'conv-1', profile: 'fake-west' },
      { action: 'thread.inactive' },
      { action: 'thread.active', conversationId: 'conv-1', profile: 'fake-west' },
    ]);
  });

  it('announces thread.inactive on unmount, so a closed thread is never the ⌥G target', async () => {
    const { root } = await mount();
    await act(async () => root.unmount());
    expect(activeActions().at(-1)).toEqual({ action: 'thread.inactive' });
  });

  it('switching to a different thread re-announces thread.active for the new one', async () => {
    const { root, el } = await mount();
    await act(async () =>
      root.render(
        <ThreadComposer
          thread={{ conversationId: 'conv-2', exchangeId: 'req-9', profile: 'fake-east' }}
          now={() => 1_000}
        />,
      ),
    );
    expect(activeActions()).toEqual([
      { action: 'thread.active', conversationId: 'conv-1', profile: 'fake-west' },
      { action: 'thread.inactive' },
      { action: 'thread.active', conversationId: 'conv-2', profile: 'fake-east' },
    ]);
    void el;
  });
});
