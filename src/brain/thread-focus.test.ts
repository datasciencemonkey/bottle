/**
 * thread-focus (#145 item 4): the hub's "focused on a thread" signal, and the view input that
 * feeds it and the composer's Send (`ThreadComposer.tsx`'s `thread.active` / `thread.inactive` /
 * `thread.send`, all under the Conversations view id).
 */
import { BrainBus } from './bus';
import type { FollowupInput, GenieFollowup } from './genie-followup';
import { createThreadFocus, routeAskToFocusedThread, wireThreadFollowup } from './thread-focus';

let bus: BrainBus;
beforeEach(() => {
  bus = new BrainBus();
});

function input(payload: unknown, from = 'hub', mogId = 'conversations', kind = 'click'): void {
  bus.dispatch({ event: 'view:input', payload: { from, input: { mogId, kind, payload } } });
}

function fakeFollowup() {
  const sends: FollowupInput[] = [];
  const followup: GenieFollowup = {
    send: async (x) => void sends.push(x),
    access: async (conversationId) => ({ conversationId, state: 'ready', text: '' }),
  };
  return { followup, sends };
}

describe('createThreadFocus (#145)', () => {
  it('[#145] holds no thread until set, then holds exactly what was set', () => {
    const focus = createThreadFocus();
    expect(focus.active()).toBeUndefined();
    focus.set({ conversationId: 'c1', profile: 'fake-west' });
    expect(focus.active()).toEqual({ conversationId: 'c1', profile: 'fake-west' });
    focus.set(undefined);
    expect(focus.active()).toBeUndefined();
  });
});

describe('wireThreadFollowup (#145)', () => {
  it('[#145] thread.active sets the focus; thread.inactive clears it', () => {
    const focus = createThreadFocus();
    wireThreadFollowup(bus, { focus, followup: fakeFollowup().followup });
    input({ action: 'thread.active', conversationId: 'c1', profile: 'fake-west' });
    expect(focus.active()).toEqual({ conversationId: 'c1', profile: 'fake-west' });
    input({ action: 'thread.inactive' });
    expect(focus.active()).toBeUndefined();
  });

  it('[#145] thread.send calls the follow-up sender with the exact question', async () => {
    const focus = createThreadFocus();
    const f = fakeFollowup();
    wireThreadFollowup(bus, { focus, followup: f.followup });
    input({
      action: 'thread.send',
      conversationId: 'c1',
      profile: 'fake-west',
      question: 'How many are there?',
    });
    await Promise.resolve();
    expect(f.sends).toEqual([
      { conversationId: 'c1', profile: 'fake-west', question: 'How many are there?' },
    ]);
  });

  it('[#145] [ARC-5] ignores input from another window, another view id or another kind', () => {
    const focus = createThreadFocus();
    wireThreadFollowup(bus, { focus, followup: fakeFollowup().followup });
    input({ action: 'thread.active', conversationId: 'c1', profile: 'fake-west' }, 'mog-x');
    input({ action: 'thread.active', conversationId: 'c1', profile: 'fake-west' }, 'hub', 'home');
    input(
      { action: 'thread.active', conversationId: 'c1', profile: 'fake-west' },
      'hub',
      'conversations',
      'ready',
    );
    expect(focus.active()).toBeUndefined();
  });

  it('[#145] a malformed payload changes nothing', () => {
    const focus = createThreadFocus();
    const f = fakeFollowup();
    wireThreadFollowup(bus, { focus, followup: f.followup });
    input({ action: 'thread.active', conversationId: '', profile: 'fake-west' });
    input({ action: 'thread.send', conversationId: 'c1', profile: '', question: 'x' });
    input({ action: 'thread.send', conversationId: 'c1', profile: 'fake-west', question: '   ' });
    expect(focus.active()).toBeUndefined();
    expect(f.sends).toEqual([]);
  });

  it('[#145] dispose stops the hub input', () => {
    const focus = createThreadFocus();
    const w = wireThreadFollowup(bus, { focus, followup: fakeFollowup().followup });
    w.dispose();
    input({ action: 'thread.active', conversationId: 'c1', profile: 'fake-west' });
    expect(focus.active()).toBeUndefined();
  });

  it('[#145] thread.check answers with the thread access, to the hub only through sendAccess', async () => {
    const focus = createThreadFocus();
    const f = fakeFollowup();
    const seen: unknown[] = [];
    wireThreadFollowup(bus, {
      focus,
      followup: {
        ...f.followup,
        access: async (conversationId) => ({
          conversationId,
          state: 'sign_in',
          text: 'Sign in to fake.example.com to ask Genie.',
        }),
      },
      sendAccess: (a) => void seen.push(a),
    });
    input({ action: 'thread.check', conversationId: 'c1' });
    await vi.waitFor(() => expect(seen).toHaveLength(1));
    expect(seen[0]).toMatchObject({ conversationId: 'c1', state: 'sign_in' });
    expect(f.sends).toEqual([]);
  });

  it('[#145] thread.check from another window, or with a blank id, is ignored', async () => {
    const f = fakeFollowup();
    const seen: unknown[] = [];
    wireThreadFollowup(bus, {
      focus: createThreadFocus(),
      followup: f.followup,
      sendAccess: (a) => void seen.push(a),
    });
    input({ action: 'thread.check', conversationId: 'c1' }, 'mog');
    input({ action: 'thread.check', conversationId: '' });
    await Promise.resolve();
    expect(seen).toEqual([]);
  });
});

describe('routeAskToFocusedThread (#145, ⌥G)', () => {
  const ask = (over: Partial<Parameters<typeof routeAskToFocusedThread>[0]> = {}) => ({
    route: 'ask',
    question: 'How many are there?',
    decision: {},
    ...over,
  });
  const focused = (hubFocused = async () => true) => {
    const focus = createThreadFocus();
    focus.set({ conversationId: 'c1', profile: 'fake-west' });
    const f = fakeFollowup();
    const release = vi.fn();
    return { focus, f, release, deps: { focus, followup: f.followup, release, hubFocused } };
  };

  it('[#145] ⌥G while the hub is focused on a thread is a follow-up in that thread', async () => {
    const t = focused();
    await expect(routeAskToFocusedThread(ask(), t.deps)).resolves.toBe(true);
    expect(t.release).toHaveBeenCalledOnce();
    expect(t.f.sends).toEqual([
      { conversationId: 'c1', profile: 'fake-west', question: 'How many are there?' },
    ]);
  });

  it('[#145] ⌥G with no focused thread (hub blurred, hidden or on another view) is not taken, and never asks the window', async () => {
    const focus = createThreadFocus();
    const f = fakeFollowup();
    const release = vi.fn();
    const hubFocused = vi.fn(async () => true);
    const deps = { focus, followup: f.followup, release, hubFocused };
    await expect(routeAskToFocusedThread(ask(), deps)).resolves.toBe(false);
    expect(release).not.toHaveBeenCalled();
    expect(hubFocused).not.toHaveBeenCalled();
    expect(f.sends).toEqual([]);
  });

  it('[#145] a missed blur never sends a ⌥G from another app into the thread: the window itself is not focused', async () => {
    const t = focused(async () => false);
    await expect(routeAskToFocusedThread(ask(), t.deps)).resolves.toBe(false);
    expect(t.release).not.toHaveBeenCalled();
    expect(t.f.sends).toEqual([]);
    // The stale focus is dropped, so no later press is taken either.
    expect(t.focus.active()).toBeUndefined();
  });

  it('[#145] a window focus check that fails counts as not focused', async () => {
    const t = focused(() => Promise.reject(new Error('fake')));
    await expect(routeAskToFocusedThread(ask(), t.deps)).resolves.toBe(false);
    expect(t.f.sends).toEqual([]);
  });

  it('[#145] a focus that moves on while the window is asked is not taken', async () => {
    const t = focused(async () => {
      t.focus.set(undefined);
      return true;
    });
    await expect(routeAskToFocusedThread(ask(), t.deps)).resolves.toBe(false);
    expect(t.f.sends).toEqual([]);
  });

  it('[#145] a dictation, or an injection-suspected ask, is never taken, even with a focused thread', async () => {
    const t = focused();
    await expect(routeAskToFocusedThread(ask({ route: 'dictation' }), t.deps)).resolves.toBe(false);
    await expect(
      routeAskToFocusedThread(ask({ decision: { injection_suspected: true } }), t.deps),
    ).resolves.toBe(false);
    expect(t.f.sends).toEqual([]);
  });
});

describe('agent threads (ADR 0121)', () => {
  const THREAD = '41111111-2222-4333-8444-000000000001';
  const MOG = '00000000-0000-4000-8000-00000000000a';

  it('[AC-C5] thread.active_agent sets the agent thread as the focus; thread.inactive clears it', () => {
    const focus = createThreadFocus();
    wireThreadFollowup(bus, { focus, followup: fakeFollowup().followup });
    input({ action: 'thread.active_agent', threadId: THREAD });
    expect(focus.active()).toEqual({ agentThreadId: THREAD });
    input({ action: 'thread.active_agent', threadId: 'nope' });
    expect(focus.active()).toEqual({ agentThreadId: THREAD });
    input({ action: 'thread.inactive' });
    expect(focus.active()).toBeUndefined();
  });

  it('[AC-C4] agent.thread_send passes the thread, the MOG, the question and the host ticket on', () => {
    const sends: unknown[] = [];
    wireThreadFollowup(bus, {
      focus: createThreadFocus(),
      followup: fakeFollowup().followup,
      agentSend: (a, ticket) => void sends.push([a, ticket]),
    });
    const send = { action: 'agent.thread_send', threadId: THREAD, mogId: MOG, question: 'And Q4?' };
    bus.dispatch({
      event: 'view:input',
      payload: {
        from: 'hub',
        input: { mogId: 'conversations', kind: 'click', payload: send },
        actionTicket: 'ticket-1',
      },
    });
    // No ticket (the host minted none): nothing, so a ⌥G latch never approves a hub send.
    input(send);
    // From another window, or with a bad shape: nothing.
    input(send, 'card-x');
    input({ ...send, question: '   ' });
    input({ ...send, mogId: 'not-a-uuid' });
    expect(sends).toEqual([
      [
        { action: 'agent.thread_send', threadId: THREAD, mogId: MOG, question: 'And Q4?' },
        'ticket-1',
      ],
    ]);
  });

  it('[AC-C5] ⌥G while the hub is focused on an agent thread is a follow-up in it, not a Genie follow-up', async () => {
    const focus = createThreadFocus();
    focus.set({ agentThreadId: THREAD });
    const f = fakeFollowup();
    const agent: [string, string][] = [];
    const release = vi.fn();
    const deps = {
      focus,
      followup: f.followup,
      agentSend: (t: string, q: string) => void agent.push([t, q]),
      release,
      hubFocused: async () => true,
    };
    await expect(
      routeAskToFocusedThread({ route: 'ask', question: 'More?', decision: {} }, deps),
    ).resolves.toBe(true);
    expect(agent).toEqual([[THREAD, 'More?']]);
    expect(f.sends).toEqual([]);
    expect(release).toHaveBeenCalledOnce();
    // A blurred hub is not a target.
    await expect(
      routeAskToFocusedThread(
        { route: 'ask', question: 'More?', decision: {} },
        { ...deps, hubFocused: async () => false },
      ),
    ).resolves.toBe(false);
    expect(focus.active()).toBeUndefined();
  });
});
