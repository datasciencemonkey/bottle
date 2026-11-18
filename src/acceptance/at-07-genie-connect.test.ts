/**
 * AT-07 "Connect and poll Genie One" (#234), section 15: the brain part. Pass: the URL has `?o=`;
 * polls are serial with the D31 cadence; IDs persist; the Beta URL is rejected.
 *
 * The poll queue ships: `ConversationManager.enqueuePoll` runs on the brain actor (ARC-2), one
 * task per conversation at a time. So G-07b runs here: concurrent callers on one conversation
 * against the #229 Genie One MCP double never have two polls open. A control case without the
 * queue shows that the double does see an overlap, so the check can fail. The G-POLL cadence
 * (G-07c) and the poll path without `progressToken` (G-07a) run on the PollActor (#31). The URL,
 * the Beta refusal and the thin client are Rust (#28, #30), in
 * `crates/mog-core/tests/at_07_genie_connect.rs`, with the `mcp_call` contract part. The live
 * run needs a person with a U2M profile (docs/tests/AT-GENIE-MCP.md).
 */
import { BrainActor } from '../brain/actor';
import { createConversationManager } from '../brain/components/conversation-manager';
import {
  GENIE_ONE_TOOLS,
  GENIE_TOOL_CONTRACTS,
  createGenieOneMcpDouble,
  scenarios,
} from '../test-support/genie-one-mcp';
import { liveFixture } from '../test-support/fixtures';
import { drain, polls, rig } from '../test-support/poll-actor-rig';
import { basePollDelayMs } from '../brain/components/poll-actor';

const HOST = 'example.cloud.databricks.com';
const WSID = '1234567890';

/** A double that holds each poll open for one macrotask, so unguarded polls overlap. */
function slowPollDouble() {
  return createGenieOneMcpDouble({
    host: HOST,
    workspaceId: WSID,
    responses: [scenarios.unknownStatus(5)],
    beforeReply: (tool) =>
      tool === 'genie_poll_response' ? new Promise((r) => setTimeout(r, 1)) : undefined,
  });
}

async function accepted(d: ReturnType<typeof createGenieOneMcpDouble>) {
  const a = await d.call('genie_ask', { question: 'What was revenue last quarter?' });
  const s = a.structuredContent as { conversation_id: string; response_id: string };
  return { conversation_id: s.conversation_id, response_id: s.response_id };
}

describe('AT-07 connect and poll Genie One', () => {
  it('[AT-22] control: unguarded concurrent poll callers overlap on the double', async () => {
    const d = slowPollDouble();
    const ref = await accepted(d);
    await Promise.all([1, 2, 3].map(() => d.call('genie_poll_response', ref)));
    expect(d.maxConcurrentPolls()).toBeGreaterThan(1);
  });

  it('[AT-07] [G-07b] concurrent poll callers on one conversation send one poll at a time', async () => {
    const d = slowPollDouble();
    const cm = createConversationManager(new BrainActor());
    const ref = await accepted(d);
    const replies = await Promise.all(
      [1, 2, 3, 4].map(() =>
        cm.enqueuePoll(ref.conversation_id, () => d.call('genie_poll_response', ref)),
      ),
    );
    expect(d.maxConcurrentPolls()).toBe(1);
    expect(d.polls()).toHaveLength(4);
    // IDs persist: every poll carries the same conversation and response IDs.
    for (const p of d.polls()) expect(p.args).toEqual(ref);
    expect(replies.every((r) => !r.isError)).toBe(true);
    // Every call fits its genie-tools.json input schema.
    expect(d.contractViolations()).toEqual([]);
  });

  it('[AT-07] [G-07b] a failed poll does not stop the next poll on the conversation', async () => {
    const d = slowPollDouble();
    const cm = createConversationManager(new BrainActor());
    const ref = await accepted(d);
    const first = cm.enqueuePoll(ref.conversation_id, async () => {
      await d.call('genie_poll_response', ref);
      throw new Error('transport lost');
    });
    const second = cm.enqueuePoll(ref.conversation_id, () => d.call('genie_poll_response', ref));
    await expect(first).rejects.toThrow('transport lost');
    expect((await second).isError).toBe(false);
    expect(d.maxConcurrentPolls()).toBe(1);
  });

  it('[AT-22] the double lists the four tools of the contract file and no other', async () => {
    const d = slowPollDouble();
    const list = await d.handle({ jsonrpc: '2.0', id: 1, method: 'tools/list' });
    const tools = (list.result as { tools: { name: string; inputSchema: unknown }[] }).tools;
    expect(tools.map((t) => t.name)).toEqual([...GENIE_ONE_TOOLS]);
    expect(GENIE_TOOL_CONTRACTS.map((t) => t.name)).toEqual([...GENIE_ONE_TOOLS]);
    for (const t of tools)
      expect(t.inputSchema).toEqual(
        GENIE_TOOL_CONTRACTS.find((c) => c.name === t.name)?.inputSchema,
      );
    // A tool outside the four is refused before it reaches a tool.
    const bad = await d.handle({
      jsonrpc: '2.0',
      id: 2,
      method: 'tools/call',
      params: { name: 'execute_sql', arguments: {} },
    });
    expect(bad.error?.code).toBe(-32602);
  });

  it('[AT-07] [G-07a] [G-07c] polls follow Table G-POLL with no progressToken', async () => {
    // The PollActor on the sim scheduler against `scenarios.unknownStatus(n)`, with real jitter.
    // Delays count from the end of the previous poll: 2 s up to 20 s, 3 s up to 60 s, 5 s up
    // to 180 s, then x1.5 to 15 s, each within +-10 %. No poll carries _meta.progressToken.
    let seed = 7;
    const random = () => ((seed = (seed * 16807) % 2147483647) - 1) / 2147483646;
    const r = rig({ responses: [scenarios.unknownStatus(50)], pollMs: 400, random });
    const target = await r.ask();
    const { outcome } = await drain(r, r.cm.poll(target));
    expect(outcome).toMatchObject({ kind: 'terminal', status: 'completed' });
    const p = polls(r);
    expect(p).toHaveLength(51);
    let prevBase: number | undefined;
    let prevEnd = target.askedAt;
    for (const x of p) {
      expect(x.args.meta).toBeUndefined();
      expect(JSON.stringify(x.args)).not.toContain('progressToken');
      const base = basePollDelayMs(prevEnd - target.askedAt, prevBase);
      const gap = x.start - prevEnd;
      expect(gap).toBeGreaterThanOrEqual(base * 0.9 - 1e-6);
      expect(gap).toBeLessThanOrEqual(base * 1.1 + 1e-6);
      prevBase = base;
      prevEnd = x.end;
    }
    // Every band of the table is reached, up to the 15 s cap.
    expect(prevBase).toBe(15_000);
    // IDs persist; nothing is asked again.
    expect(new Set(p.map((x) => JSON.stringify(x.args.args))).size).toBe(1);
    expect(r.double.asks()).toHaveLength(1);
    expect(r.double.contractViolations()).toEqual([]);
  });

  it.skip('[AT-07] [G-02] [G-03] the MOG backend config builds ?o= and refuses the Beta URL TODO(#28)', () => {
    // When #28 lands: `GenieBridge.backendFor({ kind: 'genie_one_mcp', profile })` resolves the
    // Genie One endpoint with ?o=<workspace_id>; a config with /api/2.0/mcp/genie fails to load.
    expect.unreachable('#28: the backend config is not built');
  });

  it.skip('[AT-07] [G-06b] genie_ask carries _meta.warehouse_id when the MOG has one TODO(#30)', () => {
    // When #30 lands: submit through the bridge with binding.warehouseId = 'wh-1' and read the
    // `meta` of the `mcp_call` sent to the core (src/brain/test-core.ts).
    expect.unreachable('#30: the bridge does not send mcp_call yet');
  });

  it.skip('[AT-07] live: resolve the workspace, list tools, ask and poll serially TODO(#30)', (ctx) => {
    const live = liveFixture();
    if (!live.enabled) return ctx.skip(live.reason);
    // docs/tests/AT-GENIE-MCP.md, part AT-07. Never print the token or the answer text.
    expect.unreachable('#30: the live path needs R8 McpTransport');
  });
});
