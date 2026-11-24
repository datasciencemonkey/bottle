/** ADR 0122 routing rules R-1, R-6 and R-12 to R-15. All names are plainly fake. */
import { createSimClock } from './clock';
import {
  FOLLOW_UP_WINDOW_MS,
  createFollowUpWindow,
  routeAsk,
  threadReplyMog,
  type AgentRouteInput,
} from './agent-route';
import type { OrchestratorState } from '../shared/ipc';

const input = (over: Partial<AgentRouteInput> = {}): AgentRouteInput => ({
  orchestrator: { state: 'ready' },
  ...over,
});

const NOT_READY: OrchestratorState[] = ['no_home', 'signed_out', 'no_model', 'unreachable'];

describe('agent routing (ADR 0122)', () => {
  it('[R-1] with the orchestrator ready, every question goes to the agent', () => {
    expect(routeAsk(input())).toEqual({ path: 'agent', mode: 'ask' });
  });

  it('[R-6] a question names no workspace and holds no compare word: still the agent', () => {
    expect(routeAsk(input()).path).toBe('agent');
  });

  it('[R-6] each not-ready state takes the direct path with a reason', () => {
    for (const state of NOT_READY) {
      expect(routeAsk(input({ orchestrator: { state } })), state).toEqual({
        path: 'direct',
        reason: 'not_ready',
      });
    }
    expect(routeAsk(input({ orchestrator: undefined }))).toEqual({
      path: 'direct',
      reason: 'not_ready',
    });
  });

  it('[R-8] no route offers "Ask other workspaces too"', () => {
    for (const orchestrator of [{ state: 'ready' }, { state: 'unreachable' }, undefined] as const) {
      expect(routeAsk(input({ orchestrator })), String(orchestrator?.state)).not.toHaveProperty(
        'offerOthers',
      );
    }
  });

  it('[R-12] a thread follow-up stays direct, with no reason, even when ready', () => {
    expect(routeAsk(input({ thread: true, followUp: true }))).toEqual({ path: 'direct' });
  });

  it('[R-13] [R-14] an open follow-up window sends any question to the session as a follow-up', () => {
    expect(routeAsk(input({ followUp: true }))).toEqual({ path: 'agent', mode: 'follow_up' });
    // Not ready: the follow-up cannot reach a session.
    expect(routeAsk(input({ followUp: true, orchestrator: undefined })).path).toBe('direct');
  });

  it('[R-15] a new question is a plain ask (Rust restarts a tainted session)', () => {
    expect(routeAsk(input({ followUp: false }))).toEqual({ path: 'agent', mode: 'ask' });
  });
});

describe('the follow-up window (R-13)', () => {
  it('stays open while the card is open, and for 10 min after the last answer', () => {
    const clock = createSimClock(1_000);
    const w = createFollowUpWindow(clock);
    expect(w.isOpen()).toBe(false);
    w.card(true);
    expect(w.isOpen()).toBe(true);
    clock.advance(60 * 60_000);
    expect(w.isOpen()).toBe(true);
    w.card(false);
    expect(w.isOpen()).toBe(false);
    w.answered();
    clock.advance(FOLLOW_UP_WINDOW_MS - 1);
    expect(w.isOpen()).toBe(true);
    clock.advance(1);
    expect(w.isOpen()).toBe(false);
  });

  it('[R-14] a new answer renews the window and a session end closes it', () => {
    const clock = createSimClock();
    const w = createFollowUpWindow(clock);
    w.answered();
    clock.advance(FOLLOW_UP_WINDOW_MS - 1);
    w.answered();
    clock.advance(FOLLOW_UP_WINDOW_MS - 1);
    expect(w.isOpen()).toBe(true);
    w.reset();
    expect(w.isOpen()).toBe(false);
  });
});

describe('threadReplyMog', () => {
  it('refuses a hub MOG that does not own the thread ("Not sent.")', () => {
    expect(threadReplyMog('mog-b', 'mog-a')).toBe('refused');
  });
  it('takes the hub MOG when it owns the thread, or the owner when the hub names none', () => {
    expect(threadReplyMog('mog-a', 'mog-a')).toEqual({ mogId: 'mog-a' });
    expect(threadReplyMog(undefined, 'mog-a')).toEqual({ mogId: 'mog-a' });
    expect(threadReplyMog('mog-a', undefined)).toEqual({ mogId: 'mog-a' });
    expect(threadReplyMog(undefined, undefined)).toEqual({ mogId: undefined });
  });
});
