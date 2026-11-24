/** Slice 2a routing rules R-1 to R-15 (plan Amendment 2 A2). All names are plainly fake. */
import { createSimClock } from './clock';
import {
  COMPARE_WORDS,
  FOLLOW_UP_WINDOW_MS,
  createFollowUpWindow,
  routeAsk,
  threadReplyMog,
  type AgentRouteInput,
  type RouteWorkspace,
} from './agent-route';

const home: RouteWorkspace = { profile: 'fake-home', name: 'Sales', isHome: true };
const fin: RouteWorkspace = { profile: 'fake-fin', name: 'Finance', isHome: false };
const eu: RouteWorkspace = { profile: 'fake-eu', name: 'EU data', isHome: false };
const ready = { binary: 'ready', agentProfileState: 'ready' } as const;

const input = (over: Partial<AgentRouteInput> = {}): AgentRouteInput => ({
  question: 'What was revenue last quarter?',
  genieCode: ready,
  workspaces: [home, fin],
  ...over,
});

describe('agent routing (R-1 to R-15)', () => {
  it('[R-6] a plain question takes the direct path', () => {
    expect(routeAsk(input())).toEqual({ path: 'direct', offerOthers: true });
  });

  it('[R-3] [R-4] a question that names an available non-home workspace goes to the agent', () => {
    expect(routeAsk(input({ question: 'Compare churn in sales and finance' }))).toEqual({
      path: 'agent',
      mode: 'ask',
    });
    // The name, in any case, or the profile name.
    expect(routeAsk(input({ question: 'revenue in FINANCE' })).path).toBe('agent');
    expect(routeAsk(input({ question: 'revenue in fake-fin please' })).path).toBe('agent');
  });

  it('[R-4] a name matches whole words only, and a name with spaces matches as a phrase', () => {
    expect(routeAsk(input({ question: 'the financed deals' })).path).toBe('direct');
    expect(routeAsk(input({ question: 'refinance ratio' })).path).toBe('direct');
    const two = input({ question: 'show EU   data totals', workspaces: [home, eu] });
    expect(routeAsk(two).path).toBe('agent');
  });

  it('[R-4] a name with regex characters is matched as text', () => {
    const odd: RouteWorkspace = { profile: 'fake-odd', name: 'R&D (west)', isHome: false };
    expect(
      routeAsk(input({ question: 'costs in r&d (west) now', workspaces: [home, odd] })).path,
    ).toBe('agent');
    expect(routeAsk(input({ question: 'costs in rxd', workspaces: [home, odd] })).path).toBe(
      'direct',
    );
  });

  it('[R-3] naming only the home workspace is not enough', () => {
    expect(routeAsk(input({ question: 'revenue in sales' })).path).toBe('direct');
  });

  it('[R-5] each compare word sends the question to the agent', () => {
    for (const w of COMPARE_WORDS) {
      const r = routeAsk(input({ question: `please ${w.toUpperCase()} the revenue` }));
      expect(r, w).toEqual({ path: 'agent', mode: 'ask' });
    }
    // A compare word inside another word does not count.
    expect(routeAsk(input({ question: 'Bothell store revenue' })).path).toBe('direct');
    expect(routeAsk(input({ question: 'transvestment' })).path).toBe('direct');
  });

  it('[R-2] with only home available, nothing goes to the agent and no chip shows', () => {
    const r = routeAsk(input({ question: 'compare sales and finance', workspaces: [home] }));
    expect(r).toEqual({ path: 'direct', offerOthers: false });
  });

  it('[R-1] [R-11] a question for the agent with Genie Code not ready goes direct with a reason', () => {
    for (const genieCode of [
      undefined,
      { binary: 'not_installed', agentProfileState: 'ready' } as const,
      { binary: 'wrong_version', agentProfileState: 'ready' } as const,
      { binary: 'ready', agentProfileState: 'signed_out' } as const,
      { binary: 'installing', agentProfileState: 'ready' } as const,
    ]) {
      expect(routeAsk(input({ question: 'compare sales and finance', genieCode }))).toEqual({
        path: 'direct',
        offerOthers: false,
        reason: 'not_ready',
      });
    }
  });

  it('[R-8] a plain question with the agent ready offers "Ask other workspaces too"', () => {
    expect(routeAsk(input())).toMatchObject({ path: 'direct', offerOthers: true });
    // Not ready: no chip (the click would be refused).
    expect(routeAsk(input({ genieCode: undefined }))).toMatchObject({ offerOthers: false });
  });

  it('[R-12] a thread follow-up stays direct, even with a compare word, and shows no chip', () => {
    expect(
      routeAsk(input({ question: 'compare sales and finance', thread: true, followUp: true })),
    ).toEqual({ path: 'direct', offerOthers: false });
  });

  it('[R-13] [R-14] an open follow-up window sends any question to the session as a follow-up', () => {
    expect(routeAsk(input({ followUp: true }))).toEqual({ path: 'agent', mode: 'follow_up' });
    // Not ready: the follow-up cannot reach a session.
    expect(routeAsk(input({ followUp: true, genieCode: undefined })).path).toBe('direct');
  });

  it('[R-15] a new cross-workspace question is a plain ask (Rust restarts a tainted session)', () => {
    expect(routeAsk(input({ question: 'finance vs sales', followUp: false }))).toEqual({
      path: 'agent',
      mode: 'ask',
    });
  });

  it('[R-7] the routing check stays far under 5 ms', () => {
    const many = Array.from({ length: 50 }, (_, n) => ({
      profile: `fake-${n}`,
      name: `Workspace ${n}`,
      isHome: n === 0,
    }));
    const q =
      'What did the team book last quarter across the regions and the product lines? '.repeat(10);
    const t0 = performance.now();
    for (let n = 0; n < 200; n++) routeAsk(input({ question: q, workspaces: many }));
    expect((performance.now() - t0) / 200).toBeLessThan(5);
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
