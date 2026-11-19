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
import { HUB_THREAD_ACCESS_EVENT, HUB_THREAD_STATUS_EVENT } from '../../shared/hub-conversations';

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

  it('[review] never announces thread.active while the OS window has no focus at mount time', async () => {
    const hasFocus = vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    await mount();
    expect(activeActions()).toEqual([{ action: 'thread.inactive' }]);
    hasFocus.mockRestore();
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

  describe('an error, and Try again', () => {
    const fail = (over: Record<string, unknown> = {}) =>
      tauri.emitTo('hub', HUB_THREAD_STATUS_EVENT, {
        conversationId: 'conv-1',
        requestId: 'req-2',
        phase: 'failed',
        text: 'The connection to Genie stopped.',
        question: 'How many are there?',
        ...over,
      });
    const retry = (el: HTMLElement) => el.querySelector('[data-thread-retry]') as HTMLButtonElement;

    it('shows the fixed error inline with Try again, and opens the field again', async () => {
      const { el } = await mount();
      await act(async () => {
        type(field(el), 'How many are there?');
      });
      await act(async () => void send(el).click());
      await fail();
      const alert = el.querySelector('[role="alert"]');
      expect(alert?.textContent).toContain('The connection to Genie stopped.');
      expect(retry(el).textContent).toBe('Try again');
      expect(field(el).disabled).toBe(false);
      expect(el.textContent).not.toContain('Asking Genie');
    });

    it('Try again sends the same question again on the same thread', async () => {
      const { el } = await mount();
      await act(async () => {
        type(field(el), 'How many are there?');
      });
      await act(async () => void send(el).click());
      await fail();
      await act(async () => void retry(el).click());
      expect(sendActions()).toHaveLength(2);
      expect(sendActions()[1]).toEqual({
        action: 'thread.send',
        conversationId: 'conv-1',
        profile: 'fake-west',
        question: 'How many are there?',
      });
      // The error line gives way to the working state.
      expect(el.querySelector('[role="alert"]')).toBeNull();
      expect(el.textContent).toContain('Asking Genie');
    });

    it('a failure of a voice follow-up (no typed question) offers Try again with its question', async () => {
      const { el } = await mount();
      await fail({ question: 'What about last year?' });
      await act(async () => void retry(el).click());
      expect(sendActions()[0]?.['question']).toBe('What about last year?');
    });

    it('a failure of another conversation never shows here', async () => {
      const { el } = await mount();
      await fail({ conversationId: 'conv-other' });
      expect(el.querySelector('[role="alert"]')).toBeNull();
    });

    it('shows an error when the view cannot reach the brain, and Try again resends', async () => {
      const { el } = await mount();
      tauri.invoke.mockRejectedValueOnce(new Error('fake ipc failure'));
      await act(async () => {
        type(field(el), 'How many are there?');
      });
      await act(async () => void send(el).click());
      expect(el.querySelector('[role="alert"]')?.textContent).toContain(
        'Bottle could not send this question.',
      );
      expect(retry(el)).not.toBeNull();
    });
  });

  describe('the disabled state (a workspace that is not signed in)', () => {
    const access = (state: 'ready' | 'sign_in', over: Record<string, unknown> = {}) =>
      tauri.emitTo('hub', HUB_THREAD_ACCESS_EVENT, {
        conversationId: 'conv-1',
        state,
        text: state === 'sign_in' ? 'Sign in to fake.example.com to ask Genie.' : '',
        ...over,
      });

    it('asks the brain whether the thread is signed in, on mount', async () => {
      await mount();
      expect(actions().filter((a) => a['action'] === 'thread.check')).toEqual([
        { action: 'thread.check', conversationId: 'conv-1' },
      ]);
    });

    it('disables the field and Send with the reason, and Enter sends nothing', async () => {
      const { el } = await mount();
      await access('sign_in');
      expect(el.textContent).toContain('Sign in to fake.example.com to ask Genie.');
      expect(field(el).disabled).toBe(true);
      expect(send(el).disabled).toBe(true);
      await act(async () => {
        field(el).dispatchEvent(
          new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }),
        );
      });
      expect(sendActions()).toHaveLength(0);
    });

    it('opens again when the brain says the thread is ready (signed in meanwhile)', async () => {
      const { el } = await mount();
      await access('sign_in');
      await access('ready');
      expect(field(el).disabled).toBe(false);
      expect(el.textContent).not.toContain('Sign in to');
    });

    it('asks again when the hub window gets focus', async () => {
      await mount();
      const before = actions().filter((a) => a['action'] === 'thread.check').length;
      await act(async () => void window.dispatchEvent(new Event('focus')));
      expect(actions().filter((a) => a['action'] === 'thread.check').length).toBe(before + 1);
    });

    it('ignores the access of another conversation', async () => {
      const { el } = await mount();
      await access('sign_in', { conversationId: 'conv-other' });
      expect(field(el).disabled).toBe(false);
    });
  });
});
