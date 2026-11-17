import type {
  MemoryHit,
  MemoryService,
  ProfileName,
  ProfileService,
  RouterDecision,
} from '../../shared/interfaces';
import { routerDecision } from '../../test-support/model-invoke';
import { createProfileService } from './profile-service';
import {
  dispatchDecision,
  type DispatchContext,
  type DispatchDeps,
  type DispatchOutcome,
} from './router-dispatch';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-core')).eventModule);

const MOG = '00000000-0000-4000-8000-0000000000a1';
const HOME = 'caller-profile';
const SALES = 'sales-ws';
const OTHER = 'other-ws';
const HOME_HOST = 'https://home.example.test';
const SALES_HOST = 'https://sales.example.test';
const OTHER_HOST = 'https://other.example.test';

type Target = Awaited<ReturnType<ProfileService['resolveTarget']>>;
type GateQuery = Parameters<MemoryService['gate']>[0];
type GateResult = Awaited<ReturnType<MemoryService['gate']>>;

function ctx(over: Partial<DispatchContext> = {}): DispatchContext {
  return {
    mogId: MOG,
    home: HOME,
    currentTarget: HOME,
    profiles: [
      { name: HOME, host: HOME_HOST, aliases: ['main'] },
      { name: SALES, host: SALES_HOST, aliases: ['sales', 'crm'] },
      { name: OTHER, host: OTHER_HOST, aliases: ['other'] },
    ],
    workspaceOf: (p: ProfileName) => ({ workspaceId: `ws-${p}` }),
    ...over,
  };
}

const hit = (over: Partial<MemoryHit> = {}): MemoryHit => ({
  itemId: 'item-1',
  type: 'decision',
  text: 'We keep the old pricing table.',
  score: 0.97,
  ageH: 24,
  ...over,
});

/** Doubles: a resolver and a gate that log every call in one shared order list. */
function doubles(
  opts: {
    resolve?: (hints: string[]) => Promise<Target>;
    gate?: (q: GateQuery) => Promise<GateResult>;
    wait?: (ms: number) => Promise<void>;
  } = {},
) {
  const order: string[] = [];
  const resolveCalls: { hints: string[]; mogId: string }[] = [];
  const gateCalls: { q: GateQuery; budgetMs?: number }[] = [];
  const deps: DispatchDeps = {
    profiles: {
      resolveTarget: async (hints, mogId) => {
        order.push('resolveTarget');
        resolveCalls.push({ hints, mogId });
        return (opts.resolve ?? (async () => ({ kind: 'none', suggestConnect: true }) as Target))(
          hints,
        );
      },
    },
    memory: {
      gate: async (q, budgetMs) => {
        order.push('gate');
        gateCalls.push({ q, ...(budgetMs === undefined ? {} : { budgetMs }) });
        return (
          opts.gate ?? (async () => ({ verdict: 'pass', hits: [], tier: 'local' }) as GateResult)
        )(q);
      },
    },
    ...(opts.wait ? { wait: opts.wait } : {}),
  };
  return { deps, order, resolveCalls, gateCalls };
}

const one =
  (profile: ProfileName, scope: 'question' | 'session' = 'session') =>
  async (): Promise<Target> => ({ kind: 'one', profile, scope });

const hop = (command: RouterDecision['command'], arg: string) =>
  routerDecision({
    intent: 'command',
    cleaned_text: 'Use the sales workspace from now on',
    genie_question: '',
    command,
    command_arg: arg,
  });

const never = () => new Promise<never>(() => undefined);

// ---------- hop commands (RTR-11a, AT-12) ----------

describe('router dispatch: hop commands', () => {
  it('[RTR-11a] [AT-12] "Use the sales workspace from now on" proposes a session hop through ProfileService.resolveTarget', async () => {
    const d = doubles({ resolve: one(SALES, 'session') });
    const out = await dispatchDecision(hop('switch_workspace_session', 'sales'), ctx(), d.deps);
    expect(d.resolveCalls).toEqual([{ hints: ['sales'], mogId: MOG }]);
    expect(out).toEqual({
      kind: 'hop',
      proposal: {
        command: 'switch_workspace_session',
        profile: SALES,
        host: SALES_HOST,
        scope: 'session',
        requiresUserAction: true,
      },
    });
    expect(d.gateCalls).toEqual([]);
  });

  it('[RTR-11a] [AT-12] switch_workspace_once proposes a hop for one question; the command sets the scope', async () => {
    // The resolver says "session", but the spoken command says "just this once": the command wins.
    const d = doubles({ resolve: one(SALES, 'session') });
    const out = await dispatchDecision(hop('switch_workspace_once', 'crm'), ctx(), d.deps);
    expect(out).toMatchObject({
      kind: 'hop',
      proposal: { command: 'switch_workspace_once', profile: SALES, scope: 'question' },
    });
  });

  it('[RTR-11a] [AT-12] reset_workspace proposes the home profile and needs no resolver', async () => {
    const d = doubles();
    const out = await dispatchDecision(
      hop('reset_workspace', ''),
      ctx({ currentTarget: SALES }),
      d.deps,
    );
    expect(out).toEqual({
      kind: 'hop',
      proposal: {
        command: 'reset_workspace',
        profile: HOME,
        host: HOME_HOST,
        scope: 'session',
        requiresUserAction: true,
      },
    });
    expect(d.resolveCalls).toEqual([]);
  });

  it('[RTR-11a] a hop is only a proposal: the context target does not change and nothing else is called', async () => {
    const c = ctx();
    const d = doubles({ resolve: one(SALES) });
    await dispatchDecision(hop('switch_workspace_session', 'sales'), c, d.deps);
    expect(c.currentTarget).toBe(HOME);
    expect(d.order).toEqual(['resolveTarget']);
  });

  it('[RTR-11a] [RTR-20b] fails closed when the resolver returns an unlisted profile', async () => {
    const d = doubles({ resolve: one('attacker-ws') });
    const out = await dispatchDecision(hop('switch_workspace_session', 'sales'), ctx(), d.deps);
    expect(out).toEqual({ kind: 'hop_refused', reason: 'profile_unknown' });
  });

  it('[RTR-11a] [RTR-20b] fails closed when the resolved profile is on a host outside AllowedWorkspaceHosts', async () => {
    const d = doubles({ resolve: one(SALES) });
    const out = await dispatchDecision(
      hop('switch_workspace_session', 'sales'),
      ctx({ allowedWorkspaceHosts: ['home.example.test'] }),
      d.deps,
    );
    expect(out).toEqual({ kind: 'hop_refused', reason: 'host_not_allowed' });
  });

  it('[RTR-11a] [RTR-20b] reset_workspace fails closed when the home host is not allowed', async () => {
    const d = doubles();
    const out = await dispatchDecision(
      hop('reset_workspace', ''),
      ctx({ allowedWorkspaceHosts: ['sales.example.test'] }),
      d.deps,
    );
    expect(out).toEqual({ kind: 'hop_refused', reason: 'host_not_allowed' });
  });

  it('[RTR-11a] fails closed when the resolver is not built yet or throws (TODO #164)', async () => {
    const deps: DispatchDeps = { ...doubles().deps, profiles: createProfileService() };
    const out = await dispatchDecision(hop('switch_workspace_session', 'sales'), ctx(), deps);
    expect(out).toEqual({ kind: 'hop_refused', reason: 'resolver_failed' });
  });

  it('[RTR-11a] fails closed on a split result (v1) and on a switch with no argument', async () => {
    const split = doubles({
      resolve: async () => ({ kind: 'split', parts: [{ question: 'a?', profile: SALES }] }),
    });
    expect(
      await dispatchDecision(hop('switch_workspace_session', 'sales'), ctx(), split.deps),
    ).toEqual({ kind: 'hop_refused', reason: 'split_not_supported' });

    const empty = doubles({ resolve: one(SALES) });
    expect(await dispatchDecision(hop('switch_workspace_session', ''), ctx(), empty.deps)).toEqual({
      kind: 'hop_refused',
      reason: 'no_argument',
    });
    expect(empty.resolveCalls).toEqual([]);
  });

  it('[RTR-11a] [W-11b] [AT-12] an ambiguous hint asks the user to choose among allowed profiles only', async () => {
    const d = doubles({
      resolve: async () => ({ kind: 'ambiguous', candidates: [SALES, OTHER, 'attacker-ws'] }),
    });
    const out = await dispatchDecision(
      hop('switch_workspace_session', 'ws'),
      ctx({ allowedWorkspaceHosts: ['home.example.test', 'sales.example.test'] }),
      d.deps,
    );
    // One allowed candidate is left, and the MOG still never picks it by itself.
    expect(out).toEqual({
      kind: 'choose_workspace',
      command: 'switch_workspace_session',
      scope: 'session',
      candidates: [SALES],
      requiresUserAction: true,
    });
  });

  it('[RTR-11a] [RTR-20b] an ambiguous hint with no allowed candidate fails closed', async () => {
    const d = doubles({ resolve: async () => ({ kind: 'ambiguous', candidates: [OTHER] }) });
    const out = await dispatchDecision(
      hop('switch_workspace_session', 'ws'),
      ctx({ allowedWorkspaceHosts: ['home.example.test'] }),
      d.deps,
    );
    expect(out).toEqual({ kind: 'hop_refused', reason: 'host_not_allowed' });
  });

  it('[RTR-11a] a hint with no profile offers to connect a workspace and switches nothing', async () => {
    const d = doubles();
    const out = await dispatchDecision(hop('switch_workspace_session', 'sales'), ctx(), d.deps);
    expect(out).toEqual({ kind: 'connect_workspace', requiresUserAction: true });
  });

  it('[RTR-11a] a question with target_profile_hint goes to the preview for that one question', async () => {
    const d = doubles({ resolve: one(SALES, 'session') });
    const decision = routerDecision({
      intent: 'question',
      cleaned_text: 'In crm, what was pipeline last week?',
      genie_question: 'What was pipeline last week?',
      target_profile_hint: SALES,
    });
    const out = await dispatchDecision(decision, ctx(), d.deps);
    expect(d.resolveCalls).toEqual([{ hints: [SALES], mogId: MOG }]);
    expect(out).toMatchObject({
      kind: 'preview',
      target: SALES,
      scope: 'question',
      question: 'What was pipeline last week?',
    });
  });

  it('[RTR-11a] [RTR-20b] a question whose hint fails to resolve never goes to the preview', async () => {
    const d = doubles({ resolve: one(SALES) });
    const decision = routerDecision({ intent: 'question', target_profile_hint: SALES });
    const out = await dispatchDecision(
      decision,
      ctx({ allowedWorkspaceHosts: ['home.example.test'] }),
      d.deps,
    );
    expect(out).toEqual({ kind: 'hop_refused', reason: 'host_not_allowed' });
  });
});

// ---------- memory questions (RTR-18a, RTR-18b, AT-35) ----------

const memoryQuestion = (over: Partial<RouterDecision> = {}) =>
  routerDecision({
    intent: 'memory_question',
    cleaned_text: 'What did we decide about the pricing table yesterday?',
    genie_question: 'What did we decide about the pricing table yesterday?',
    ...over,
  });

describe('router dispatch: memory questions', () => {
  it('[RTR-18a] [AT-35] memory_question calls the RecallGate first and answers from memory when it passes', async () => {
    const hits = [hit()];
    const d = doubles({
      gate: async () => ({ verdict: 'answer_from_memory', hits, tier: 'local' }),
    });
    const out = await dispatchDecision(memoryQuestion(), ctx(), d.deps);
    expect(d.gateCalls).toEqual([
      {
        q: {
          mogId: MOG,
          text: 'What did we decide about the pricing table yesterday?',
          intent: 'memory_question',
          target: { workspaceId: `ws-${HOME}` },
        },
        budgetMs: 300,
      },
    ]);
    expect(out).toEqual({ kind: 'memory_answer', hits, tier: 'local' });
  });

  it('[RTR-18a] [AT-35] the gate runs against the hinted workspace, after resolution and before any send', async () => {
    const d = doubles({
      resolve: one(SALES),
      gate: async () => ({ verdict: 'answer_from_memory', hits: [hit()], tier: 'local' }),
    });
    await dispatchDecision(memoryQuestion({ target_profile_hint: SALES }), ctx(), d.deps);
    expect(d.order).toEqual(['resolveTarget', 'gate']);
    expect(d.gateCalls[0]?.q.target).toEqual({ workspaceId: `ws-${SALES}` });
  });

  it('[RTR-18a] only memory_question calls the RecallGate', async () => {
    const d = doubles();
    await dispatchDecision(routerDecision({ intent: 'question' }), ctx(), d.deps);
    expect(d.gateCalls).toEqual([]);
  });

  it('[RTR-18b] [AT-35] if the gate passes, the utterance continues as a question at the preview', async () => {
    const d = doubles({ gate: async () => ({ verdict: 'pass', hits: [], tier: 'local' }) });
    const out = (await dispatchDecision(memoryQuestion(), ctx(), d.deps)) as Extract<
      DispatchOutcome,
      { kind: 'preview' }
    >;
    expect(out.kind).toBe('preview');
    expect(out.decision.intent).toBe('question');
    expect(out).toMatchObject({
      target: HOME,
      scope: 'current',
      question: 'What did we decide about the pricing table yesterday?',
      recall: [],
      gate: 'pass',
    });
  });

  it('[RTR-18b] [AT-35] augment continues at the preview and carries the recall hits', async () => {
    const hits = [hit({ score: 0.6 })];
    const d = doubles({ gate: async () => ({ verdict: 'augment', hits, tier: 'local' }) });
    const out = await dispatchDecision(memoryQuestion(), ctx(), d.deps);
    expect(out).toMatchObject({ kind: 'preview', recall: hits, gate: 'augment' });
  });

  it('[RTR-18b] [AT-35] a gate error continues at the preview (TODO #200 builds the gate)', async () => {
    const d = doubles({
      gate: async () => {
        throw new Error('gate down');
      },
    });
    const out = await dispatchDecision(memoryQuestion(), ctx(), d.deps);
    expect(out).toMatchObject({ kind: 'preview', gate: 'error', recall: [] });
  });

  it('[RTR-18b] [AT-35] a gate that does not answer in the 300 ms budget continues at the preview', async () => {
    const waits: number[] = [];
    const d = doubles({
      gate: never,
      wait: async (ms) => {
        waits.push(ms);
      },
    });
    const out = await dispatchDecision(memoryQuestion(), ctx(), d.deps);
    expect(waits).toEqual([300]);
    expect(out).toMatchObject({ kind: 'preview', gate: 'timeout', recall: [] });
  });

  it('[RTR-18b] [AT-35] the default timer ends the gate wait at 300 ms', async () => {
    vi.useFakeTimers();
    try {
      const d = doubles({ gate: never });
      const pending = dispatchDecision(memoryQuestion(), ctx(), d.deps);
      await vi.advanceTimersByTimeAsync(299);
      let done = false;
      void pending.then(() => (done = true));
      await Promise.resolve();
      expect(done).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      expect(await pending).toMatchObject({ kind: 'preview', gate: 'timeout' });
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('[RTR-18b] [AT-35] a gate that answers in time leaves no timer behind', async () => {
    vi.useFakeTimers();
    try {
      const d = doubles();
      expect(await dispatchDecision(memoryQuestion(), ctx(), d.deps)).toMatchObject({
        gate: 'pass',
      });
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('[RTR-18b] a workspace lookup error counts as a gate error and continues at the preview', async () => {
    const d = doubles();
    const out = await dispatchDecision(
      memoryQuestion(),
      ctx({
        workspaceOf: () => {
          throw new Error('no workspace');
        },
      }),
      d.deps,
    );
    expect(out).toMatchObject({ kind: 'preview', gate: 'error' });
  });

  it('[RTR-18b] answer_from_memory with no hits does not answer, so the flow continues at the preview', async () => {
    const d = doubles({
      gate: async () => ({ verdict: 'answer_from_memory', hits: [], tier: 'local' }),
    });
    const out = await dispatchDecision(memoryQuestion(), ctx(), d.deps);
    expect(out).toMatchObject({ kind: 'preview', gate: 'pass' });
  });
});

// ---------- binding rules ----------

describe('router dispatch: binding rules', () => {
  it('[RTR-11a] [RTR-18a] an injection-flagged decision proposes nothing and calls nothing', async () => {
    const d = doubles({ resolve: one(SALES) });
    const flagged = await dispatchDecision(
      { ...hop('switch_workspace_session', 'sales'), injection_suspected: true },
      ctx(),
      d.deps,
    );
    const flaggedMemory = await dispatchDecision(
      memoryQuestion({ injection_suspected: true }),
      ctx(),
      d.deps,
    );
    expect(flagged).toEqual({ kind: 'none', reason: 'injection' });
    expect(flaggedMemory).toEqual({ kind: 'none', reason: 'injection' });
    expect(d.order).toEqual([]);
  });

  it('[RTR-18a] dictation bypasses the agent: no resolver, no gate, no preview', async () => {
    const d = doubles();
    const out = await dispatchDecision(
      routerDecision({ intent: 'dictation', genie_question: '', target_profile_hint: SALES }),
      ctx(),
      d.deps,
    );
    expect(out).toEqual({ kind: 'none', reason: 'not_agent' });
    expect(d.order).toEqual([]);
  });

  it('[RTR-11a] other commands are left to the command grammar (TODO #143)', async () => {
    const d = doubles();
    const out = await dispatchDecision(hop('new_topic', ''), ctx(), d.deps);
    expect(out).toEqual({ kind: 'none', reason: 'other_command' });
  });

  it('[RTR-11a] [RTR-18b] no outcome holds a token field', async () => {
    const d = doubles({
      resolve: one(SALES),
      gate: async () => ({ verdict: 'augment', hits: [hit()], tier: 'local' }),
    });
    const outs = [
      await dispatchDecision(hop('switch_workspace_session', 'sales'), ctx(), d.deps),
      await dispatchDecision(memoryQuestion(), ctx(), d.deps),
    ];
    for (const o of outs) expect(JSON.stringify(o)).not.toMatch(/token/i);
  });
});
