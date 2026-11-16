/**
 * AT-12 "Switch workspace safely" (#233), section 15. Gate: live.
 *
 * The AT script: hop for one question and for a session during an open conversation and a
 * session. Try an ambiguous name. Pass: old conversations stay pinned, home processing is
 * unchanged and disclosed, ambiguity needs a user choice, the MOG aliases match first, and a
 * parked conversation takes a follow-up at 29 min and not at 31 min. (v1) Lakebase stores a
 * pointer for B.
 *
 * Runs today on shipped code: the router step hop commands and part checks (#57), the MOG badges
 * (#50) and the MoodMapper (#9). The hop itself is not built: target resolution (#164), hop
 * execution, banner and home processing (#165), conversation parking and pinning (#145), and v1
 * split and privacy (#146, #218). Those tests stay `it.skip` with the feature issue, so they never
 * count as a pass. The live run needs a person: docs/tests/AT-SIGNIN-WORKSPACE-HOP.md.
 */
import type { CaptureState } from '../shared/interfaces';
import { createSimClock } from '../brain/clock';
import { createGenieBridge } from '../brain/components/genie-bridge';
import { mapBadges, type BadgeSources } from '../brain/components/mog-badges';
import { mapMood, type MoodSources } from '../brain/components/mood-mapper';
import { createProfileService } from '../brain/components/profile-service';
import { runRouterStep, type RouterInput } from '../brain/components/router-step';
import { createGenieOneMcpDouble } from '../test-support/genie-one-mcp';
import { createRouterModelDouble, type RouterReply } from '../test-support/model-invoke';

vi.mock('@tauri-apps/api/core', async () => (await import('../brain/test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../brain/test-core')).eventModule);

const MOG = '00000000-0000-4000-8000-0000000000a1';
const HOME = 'bottle-home';
const SALES = 'bottle-sales';
const HOME_HOST = 'https://home.example.test';
const SALES_HOST = 'https://sales.example.test';
const MIN = 60_000;

const input: RouterInput = {
  profile: HOME,
  speakerTrack: 'you',
  captureMode: 'ptt',
  pendingClarification: '',
  recentTopics: [],
  activeMogs: [{ id: MOG, name: 'Finance' }],
  profiles: [
    { name: HOME, host: HOME_HOST, aliases: ['main'] },
    { name: SALES, host: SALES_HOST, aliases: ['sales', 'crm'] },
  ],
  currentTarget: HOME,
};

async function route(reply: RouterReply, text: string, inp: RouterInput = input) {
  const double = createRouterModelDouble([reply]);
  const decision = await runRouterStep(text, MOG, inp, { invoke: double.invoke });
  return { decision, calls: double.calls };
}

const idle: CaptureState = { recording: false, paused: false, othersTrack: 'off' };

function badges(hop?: BadgeSources['hop']) {
  return mapBadges({
    profile: HOME,
    host: HOME_HOST,
    backend: { kind: 'genie_one_mcp', profile: HOME },
    lifecycle: 'ready',
    ...(hop ? { hop } : {}),
  });
}

// ---------- Runs today: hop commands from the router step (RTR-11a, RTR-11b) ----------

describe('AT-12 hop commands (router step)', () => {
  it('[AT-12] [RTR-11a] "Use the sales workspace from now on" returns switch_workspace_session', async () => {
    const { decision } = await route(
      {
        decision: {
          intent: 'command',
          cleaned_text: 'Use the sales workspace from now on',
          genie_question: '',
          command: 'switch_workspace_session',
          command_arg: 'sales',
        },
      },
      'Use the sales workspace from now on',
    );
    expect(decision.intent).toBe('command');
    expect(decision.command).toBe('switch_workspace_session');
    expect(decision.command_arg).toBe('sales');
  });

  it('[AT-12] [RTR-11a] a one-question hop returns switch_workspace_once and a known hint', async () => {
    const { decision } = await route(
      {
        decision: {
          intent: 'question',
          cleaned_text: 'In crm, what was pipeline last week?',
          genie_question: 'What was pipeline last week?',
          target_profile_hint: 'CRM',
        },
      },
      'In crm, what was pipeline last week?',
    );
    // The alias "CRM" resolves to the profile name; the client never invents a profile.
    expect(decision.target_profile_hint).toBe(SALES);

    const unknown = await route(
      { decision: { intent: 'question', target_profile_hint: 'attacker-workspace' } },
      'What was pipeline?',
    );
    expect(unknown.decision.target_profile_hint).toBe('');

    const once = await route(
      {
        decision: {
          intent: 'command',
          genie_question: '',
          command: 'switch_workspace_once',
          command_arg: 'sales',
        },
      },
      'Ask sales just this once',
    );
    expect(once.decision.command).toBe('switch_workspace_once');
  });

  it('[AT-12] [RTR-11b] parts of unequal length or more than 3 parts are ignored', async () => {
    const unequal = await route(
      {
        decision: {
          part_questions: ['Revenue?', 'Pipeline?'],
          part_profile_hints: [HOME],
        },
      },
      'Revenue here and pipeline in sales?',
    );
    expect(unequal.decision.part_questions).toEqual([]);
    expect(unequal.decision.part_profile_hints).toEqual([]);

    const four = await route(
      {
        decision: {
          part_questions: ['a?', 'b?', 'c?', 'd?'],
          part_profile_hints: [HOME, SALES, HOME, SALES],
        },
      },
      'Four questions',
    );
    expect(four.decision.part_questions).toEqual([]);

    const two = await route(
      {
        decision: {
          part_questions: ['Revenue?', 'Pipeline?'],
          part_profile_hints: ['main', 'crm'],
        },
      },
      'Revenue here and pipeline in sales?',
    );
    expect(two.decision.part_profile_hints).toEqual([HOME, SALES]);
  });

  it('[AT-12] [W-21a] during a hop the router model call goes to the home profile (router part)', async () => {
    // TODO(#165): STT, cleanup, writer, embedding and main agent calls are in the pending test below.
    const { calls } = await route(
      { decision: { target_profile_hint: SALES } },
      'What was pipeline last week?',
      { ...input, currentTarget: SALES },
    );
    expect(calls).toHaveLength(1);
    expect(calls[0]?.profile).toBe(HOME);
    expect(calls[0]?.role).toBe('router');
  });
});

// ---------- Runs today: badges and mood during a hop (MOG-07, W-11a, UX-39a, UX-39b) ----------

describe('AT-12 hop badges', () => {
  it('[AT-12] [MOG-07] [W-11a] [UX-39a] the badges name the hop target and its scope', () => {
    for (const scope of ['question', 'session'] as const) {
      const b = badges({ profile: SALES, host: SALES_HOST, scope });
      const identity = b.find((x) => x.kind === 'identity');
      expect(identity?.text).toBe(`${SALES} · Genie One`);
      expect(identity?.detail).toBe(SALES_HOST);
      const hop = b.find((x) => x.kind === 'hop');
      expect(hop?.text).toBe(`Hop: ${SALES} (this ${scope})`);
      // The hop badge shows without hover, so the user sees the hop at all times.
      expect(hop?.hoverOnly).toBe(false);
    }
    const home = badges();
    expect(home.find((x) => x.kind === 'identity')?.text).toBe(`${HOME} · Genie One`);
    expect(home.some((x) => x.kind === 'hop')).toBe(false);
  });

  it('[AT-12] [UX-39b] a hop sets the hop overlay and never changes the mood', () => {
    const states: MoodSources[] = [
      { mog: { lifecycle: 'ready', since: 0 }, capture: idle, requests: [] },
      {
        mog: { lifecycle: 'ready', since: 0 },
        capture: idle,
        requests: [{ phase: 'working', at: 10 }],
      },
      { mog: { lifecycle: 'login_required', since: 0 }, capture: idle, requests: [] },
      { mog: { lifecycle: 'dozing', since: 0 }, capture: idle, requests: [] },
    ];
    for (const s of states) {
      const plain = mapMood(MOG, s, 100);
      const hopped = mapMood(MOG, { ...s, mog: { ...s.mog, hop: SALES } }, 100);
      expect(hopped.mood).toBe(plain.mood);
      expect(hopped.overlays.hop).toBe(SALES);
      expect(plain.overlays.hop).toBeUndefined();
    }
  });
});

// ---------- Pending: target resolution (W-11a, W-11b, W-11c) ----------

describe('AT-12 hop targets (ProfileService.resolveTarget)', () => {
  it.skip('[AT-12] [W-11c] MOG aliases match first, then profile names, then hosts TODO(#164)', async () => {
    // TODO(#164): profiles `sales` (host sales-a) and `bottle-sales` with the MOG alias "sales".
    const t7 = createProfileService();
    await expect(t7.resolveTarget(['sales'], MOG)).resolves.toEqual({
      kind: 'one',
      profile: SALES,
      scope: 'question',
    });
    await expect(t7.resolveTarget(['sales.example.test'], MOG)).resolves.toMatchObject({
      kind: 'one',
      profile: SALES,
    });
    throw new Error(
      'TODO(#164): assert an alias beats a profile name with the same text, and the first level with exactly one match wins',
    );
  });

  it.skip('[AT-12] [W-11b] two matches need a user choice; the MOG never picks TODO(#164)', async () => {
    // TODO(#164): two profiles whose names both match "prod" at the same level.
    const t7 = createProfileService();
    const r = await t7.resolveTarget(['prod'], MOG);
    expect(r.kind).toBe('ambiguous');
    if (r.kind === 'ambiguous') expect(r.candidates.length).toBeGreaterThanOrEqual(2);
    await expect(t7.resolveTarget(['no-such-workspace'], MOG)).resolves.toEqual({
      kind: 'none',
      suggestConnect: true,
    });
    throw new Error(
      'TODO(#164): assert the MOG shows the "Which workspace?" chip and sends no ask',
    );
  });

  it.skip('[AT-12] [W-11a] a hop applies to one question or to one session TODO(#164)', async () => {
    const t7 = createProfileService();
    // TODO(#164): "ask sales" is a question hop; "use sales from now on" is a session hop.
    const once = await t7.resolveTarget(['sales'], MOG);
    expect(once).toMatchObject({ kind: 'one', scope: 'question' });
    expect(t7.home(MOG)).toBe(HOME);
    throw new Error(
      'TODO(#164): assert "use sales from now on" gives scope session and the next question after a question hop goes home',
    );
  });
});

// ---------- Pending: pinning and parking (G-26a..d, G-27a, G-27b) ----------

describe('AT-12 pinned and parked conversations', () => {
  it.skip('[AT-12] [G-26c] [G-26d] a parked conversation takes a follow-up at 29 min and not at 31 min TODO(#145)', () => {
    const clock = createSimClock();
    const bridge = createGenieBridge();
    const ask = (question: string, conversationId?: string) =>
      bridge.submit({
        opId: `op-${clock.now()}`,
        mogId: MOG,
        binding: { kind: 'genie_one_mcp', profile: HOME },
        question,
        ...(conversationId ? { conversationId } : {}),
        tags: { app: 'bottle', mog: MOG, op: 'genie' },
      });
    // TODO(#145): the bridge takes `clock`; ask in HOME (c1), then hop to SALES, so c1 is parked.
    ask('What was revenue last quarter?');
    expect(bridge.pinnedProfile('c1')).toBe(HOME);
    clock.advance(29 * MIN);
    ask('And the quarter before?', 'c1');
    clock.advance(2 * MIN);
    ask('And the year before?', 'c1');
    throw new Error(
      'TODO(#145): assert the 29 min follow-up is accepted in c1 and the 31 min follow-up gets a new conversation ID',
    );
  });

  it.skip('[AT-12] [G-26a] [G-26b] reuse rules R0-R5 and one conversation per meeting session TODO(#145)', () => {
    const clock = createSimClock();
    // TODO(#145): a gap over 10 min, a topic change, a workspace change, a new session, 20 turns or
    // a user reset each start a new conversation; two meeting sessions have two conversation IDs.
    createGenieBridge().submit({
      opId: 'op-1',
      mogId: MOG,
      binding: { kind: 'genie_one_mcp', profile: HOME },
      question: 'What was revenue?',
      tags: { app: 'bottle', mog: MOG, op: 'genie' },
    });
    clock.advance(10 * MIN + 1);
    throw new Error(
      'TODO(#145): assert each of R0-R5 starts a new conversation, and two meeting sessions get two conversation IDs',
    );
  });

  it.skip('[AT-12] [G-27a] [G-27b] a follow-up to an old card goes to its pinned workspace, after a restart too TODO(#145)', () => {
    const home = createGenieOneMcpDouble({ host: HOME_HOST, workspaceId: '1001' });
    const sales = createGenieOneMcpDouble({ host: SALES_HOST, workspaceId: '2002' });
    const bridge = createGenieBridge();
    // TODO(#145): ask in HOME, hop to SALES, restart the brain, follow up on the HOME card.
    expect(bridge.pinnedProfile('c-home')).toBe(HOME);
    expect(home.asks()).toHaveLength(2);
    expect(sales.asks()).toHaveLength(0);
    throw new Error(
      'TODO(#145): assert the pin is read back from the local store after the restart',
    );
  });
});

// ---------- Pending: hop execution (W-12a, W-20, W-21a, W-21b) ----------

describe('AT-12 hop execution', () => {
  it.skip('[AT-12] [W-12a] [W-12b] after a hop new questions start new conversations in the target and open ones stay put TODO(#165)', () => {
    const sales = createGenieOneMcpDouble({ host: SALES_HOST, workspaceId: '2002' });
    // TODO(#165): hop to SALES, ask two new questions; both conversation IDs belong to SALES.
    expect(sales.asks()).toHaveLength(2);
    expect(createGenieBridge().workspaceLine(SALES)).toBe(
      `Asking Genie in ${SALES_HOST} (profile ${SALES})`,
    );
    throw new Error(
      'TODO(#165): assert both new conversation IDs belong to SALES and the open HOME conversation keeps its workspace (W-12b)',
    );
  });

  it.skip('[AT-12] [W-20] the hop banner shows before the first ask in the target TODO(#165)', () => {
    const sales = createGenieOneMcpDouble({ host: SALES_HOST, workspaceId: '2002' });
    // TODO(#165): hop to SALES and ask; the Appendix E banner shows before the first genie_ask.
    expect(createGenieBridge().operations(MOG)).toEqual([]);
    expect(sales.asks()).toHaveLength(1);
    throw new Error(
      `TODO(#165): assert the banner "Asking in ${SALES}. Context from ${HOME} is not shared." shows before the ask`,
    );
  });

  it.skip('[AT-12] [W-21a] STT, cleanup, writer, embedding and main agent calls use the home profile TODO(#165)', () => {
    // TODO(#165): run one hopped question end to end with model doubles and read the trace.
    const bridge = createGenieBridge();
    bridge.submit({
      opId: 'op-1',
      mogId: MOG,
      binding: { kind: 'genie_one_mcp', profile: SALES },
      question: 'What was pipeline last week?',
      tags: { app: 'bottle', mog: MOG, op: 'genie' },
    });
    throw new Error(
      `TODO(#165): assert every asr, cleanup, router, writer, embed and agent call names ${HOME}`,
    );
  });

  it.skip('[AT-12] [W-21b] the data panel says the home workspace processes hopped questions TODO(#165)', () => {
    // TODO(#165): render the hub data panel during a hop to SALES.
    expect(createProfileService().home(MOG)).toBe(HOME);
    throw new Error(
      'TODO(#165): assert the data panel shows "Questions about workspace B are processed in your home workspace."',
    );
  });
});

// ---------- Pending, v1: split and cross-workspace privacy ----------

describe('AT-12 (v1) split and cross-workspace processing', () => {
  it.skip('[AT-12] [W-13a] [W-13b] [W-13c] [G-50] a split asks 3 parts at most, labels each part and never adds numbers TODO(#166) TODO(#146)', async () => {
    const t7 = createProfileService();
    const r = await t7.resolveTarget(['main', 'crm'], MOG);
    expect(r.kind).toBe('split');
    if (r.kind === 'split') expect(r.parts.map((p) => p.profile)).toEqual([HOME, SALES]);
    throw new Error(
      'TODO(#166) TODO(#146): assert a 4th part is a chip, "Part 2 (sales): <reason>" on failure, numbers verbatim, one genie_exchange row per part with one split_group_id',
    );
  });

  it.skip('[AT-12] [SEC-18] [SEC-29a] [SEC-29b] target processing under MDM, and a merged split answer stays local TODO(#218)', async () => {
    const t7 = createProfileService();
    await expect(t7.resolveTarget(['main', 'crm'], MOG)).resolves.toMatchObject({ kind: 'split' });
    throw new Error(
      'TODO(#218): with RequireTargetWorkspaceProcessing=true assert ASR and SLM calls go to the target; assert the merged answer has cross_workspace, never syncs and never enters a preamble',
    );
  });
});
