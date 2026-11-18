/**
 * A PollActor rig for tests (#31): the ConversationManager on the sim scheduler, against the
 * #229 Genie One MCP double through a fake `mcp_call`. No wall time and no live workspace.
 */
import { BrainActor } from '../brain/actor';
import { createConversationManager } from '../brain/components/conversation-manager';
import {
  GIVE_UP_MS,
  createInflightAsks,
  createMemoryPollJournal,
  type McpCallArgs,
  type McpCallReply,
  type PollOutcome,
  type PollTarget,
  type PollUpdate,
} from '../brain/components/poll-actor';
import { createGenieOneMcpDouble, scenarios, type ScriptedResponse } from './genie-one-mcp';
import { createSimScheduler } from './sim-scheduler';

export const HOST = 'example.cloud.databricks.com';
export const WSID = '1234567890';
export const PROFILE = 'test-profile';
export const MOG = '00000000-0000-4000-8000-0000000000a1';

export interface Rig {
  sched: ReturnType<typeof createSimScheduler>;
  double: ReturnType<typeof createGenieOneMcpDouble>;
  cm: ReturnType<typeof createConversationManager>;
  journal: ReturnType<typeof createMemoryPollJournal>;
  /** Every `mcp_call` the actor sent, with sim start and end times. */
  sent: { args: McpCallArgs; start: number; end: number }[];
  ask(): Promise<PollTarget>;
}

export function rig(
  opts: {
    responses?: ScriptedResponse[];
    random?: () => number;
    /** Sim time that the double holds each poll open. */
    pollMs?: number;
    /** Replace the reply of poll number `n` (1-based) with a rejection. */
    reject?: (n: number) => unknown;
  } = {},
): Rig {
  const sched = createSimScheduler();
  const double = createGenieOneMcpDouble({
    host: HOST,
    workspaceId: WSID,
    responses: opts.responses ?? [scenarios.unknownStatus(3)],
    beforeReply: (tool) =>
      tool === 'genie_poll_response' && opts.pollMs ? sched.sleep(opts.pollMs) : undefined,
  });
  const journal = createMemoryPollJournal();
  const sent: Rig['sent'] = [];
  let polls = 0;
  const call = async (args: McpCallArgs): Promise<McpCallReply> => {
    const row = { args, start: sched.clock.now(), end: sched.clock.now() };
    sent.push(row);
    try {
      if (args.tool === 'genie_poll_response') {
        const bad = opts.reject?.(++polls);
        if (bad !== undefined) throw bad;
      }
      return await double.call(args.tool, args.args);
    } finally {
      row.end = sched.clock.now();
    }
  };
  const cm = createConversationManager(new BrainActor(), {
    call,
    now: () => sched.clock.now(),
    sleep: (ms, signal) => sched.sleep(ms, signal),
    random: opts.random ?? (() => 0.5),
    journal,
    inflight: createInflightAsks(),
  });
  return {
    sched,
    double,
    cm,
    journal,
    sent,
    async ask() {
      const a = await double.call('genie_ask', { question: 'What was revenue last quarter?' });
      const s = a.structuredContent as { conversation_id: string; response_id: string };
      return {
        opId: `op-${s.response_id}`,
        mogId: MOG,
        profile: PROFILE,
        host: HOST,
        conversationId: s.conversation_id,
        responseId: s.response_id,
        askedAt: sched.clock.now(),
      };
    },
  };
}

export interface Run {
  updates: PollUpdate[];
  outcome?: PollOutcome;
  error?: unknown;
  ended: boolean;
}

/** Collect a poll loop in the background. */
export function collect(loop: AsyncGenerator<PollUpdate, PollOutcome, undefined>): Run {
  const run: Run = { updates: [], ended: false };
  void (async () => {
    try {
      for (;;) {
        const x = await loop.next();
        if (x.done) {
          run.outcome = x.value;
          break;
        }
        run.updates.push(x.value);
      }
    } catch (e) {
      run.error = e;
    } finally {
      run.ended = true;
    }
  })();
  return run;
}

/** Run a poll loop to its end on sim time. */
export async function drain(
  r: Rig,
  loop: AsyncGenerator<PollUpdate, PollOutcome, undefined>,
  untilMs = GIVE_UP_MS + 60_000,
): Promise<{ updates: PollUpdate[]; outcome: PollOutcome }> {
  const run = collect(loop);
  await r.sched.runUntil(untilMs, { settle: true });
  await new Promise((res) => setTimeout(res, 0));
  if (run.error !== undefined) throw run.error;
  if (!run.ended || !run.outcome) throw new Error('the poll loop did not end');
  return { updates: run.updates, outcome: run.outcome };
}

/** The `genie_poll_response` calls the actor sent. */
export const polls = (r: Rig) => r.sent.filter((s) => s.args.tool === 'genie_poll_response');

/** Item `i` of `a`; throws when it is missing. */
export function nth<T>(a: readonly T[], i: number): T {
  const x = a[i];
  if (x === undefined) throw new Error(`no item ${i} of ${a.length}`);
  return x;
}
