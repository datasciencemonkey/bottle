// Self-tests of the Genie One MCP test double (#229, AT-22). They check the double, not the bridge,
// so they carry only the harness ID AT-22.
import { BUSY_TEXT, GENIE_ONE_TOOLS, createGenieOneMcpDouble, scenarios } from './genie-one-mcp';

const HOST = 'example.cloud.databricks.com';
const WSID = '1234567890';

function ask(
  d: ReturnType<typeof createGenieOneMcpDouble>,
  question: string,
  conversation_id?: string,
) {
  return d.call('genie_ask', conversation_id ? { question, conversation_id } : { question });
}

function ids(r: { structuredContent: unknown }) {
  const s = r.structuredContent as { conversation_id: string; response_id: string };
  return { conversation_id: s.conversation_id, response_id: s.response_id };
}

describe('Genie One MCP double', () => {
  it('[AT-22] answers initialize and lists the Genie One tools over JSON-RPC', async () => {
    const d = createGenieOneMcpDouble({ host: HOST, workspaceId: WSID });
    const init = await d.handle({ jsonrpc: '2.0', id: 1, method: 'initialize', params: {} });
    expect(init).toMatchObject({
      jsonrpc: '2.0',
      id: 1,
      result: { serverInfo: { name: 'genie_one_mcp' } },
    });
    const list = await d.handle({ jsonrpc: '2.0', id: 2, method: 'tools/list' });
    const names = (list.result as { tools: { name: string }[] }).tools.map((t) => t.name);
    expect(names).toEqual([...GENIE_ONE_TOOLS]);
    expect(names).toContain('genie_ask');
    expect(names).toContain('genie_poll_response');
  });

  it('[AT-22] rejects an unknown method and an unknown tool with JSON-RPC errors', async () => {
    const d = createGenieOneMcpDouble({ host: HOST, workspaceId: WSID });
    const bad = await d.handle({ jsonrpc: '2.0', id: 3, method: 'nope' });
    expect(bad.error?.code).toBe(-32601);
    const tool = await d.handle({
      jsonrpc: '2.0',
      id: 4,
      method: 'tools/call',
      params: { name: 'x', arguments: {} },
    });
    expect(tool.error?.code).toBe(-32602);
  });

  it('[AT-22] normal answer: ask is accepted, polls progress, then completes with a deep link', async () => {
    const d = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: WSID,
      responses: [scenarios.normalAnswer()],
    });
    const a = await ask(d, 'What was revenue last quarter?');
    expect(a.isError).toBe(false);
    expect(a.httpStatus).toBe(200);
    expect(a.structuredContent).toMatchObject({ status: 'in_progress' });
    const ref = ids(a);
    const p1 = await d.call('genie_poll_response', ref);
    expect(p1.structuredContent).toMatchObject({
      status: 'in_progress',
      progress_steps: ['Reading the tables'],
    });
    const p2 = await d.call('genie_poll_response', ref);
    const done = p2.structuredContent as Record<string, unknown>;
    expect(done.status).toBe('completed');
    expect(done.final_answer).toContain('**$1.2M**');
    expect(done.query_items).toEqual([{ item_id: 'q1', sql: 'SELECT 1' }]);
    expect(done.deep_link).toBe(
      `https://${HOST}/one/chat/threads/${ref.conversation_id}?o=${WSID}&utm_source=genie_chat_mcp`,
    );
    expect(JSON.parse(p2.text)).toEqual(done);
  });

  it('[AT-22] a poll after the terminal status repeats the terminal status', async () => {
    const d = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: WSID,
      responses: [scenarios.normalAnswer()],
    });
    const ref = ids(await ask(d, 'q'));
    await d.call('genie_poll_response', ref);
    await d.call('genie_poll_response', ref);
    const p3 = await d.call('genie_poll_response', ref);
    expect(p3.structuredContent).toMatchObject({ status: 'completed' });
  });

  it('[AT-22] busy conversation: a second ask returns the live busy text as an isError result', async () => {
    const d = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: WSID,
      responses: [scenarios.normalAnswer()],
    });
    const first = ids(await ask(d, 'first'));
    const second = await ask(d, 'second', first.conversation_id);
    expect(second.isError).toBe(true);
    expect(second.httpStatus).toBe(200);
    expect(second.text).toBe(BUSY_TEXT);
    expect(second.text).toContain('stream closed before response.created');
    expect(second.rpcError).toBeUndefined();
    expect(d.asks()).toHaveLength(1);
    // The first response is still pollable with the same IDs.
    const p = await d.call('genie_poll_response', first);
    expect(p.isError).toBe(false);
  });

  it('[AT-22] busy conversation: the JSON-RPC envelope variant returns an rpcError', async () => {
    const d = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: WSID,
      busyEnvelope: 'jsonrpc',
      responses: [scenarios.normalAnswer()],
    });
    const first = ids(await ask(d, 'first'));
    const second = await ask(d, 'second', first.conversation_id);
    expect(second.isError).toBe(true);
    expect(second.rpcError).toMatchObject({ message: BUSY_TEXT });
  });

  it('[AT-22] the conversation takes a new ask after the response is terminal', async () => {
    const d = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: WSID,
      responses: [scenarios.normalAnswer(), scenarios.normalAnswer()],
    });
    const first = ids(await ask(d, 'first'));
    await d.call('genie_poll_response', first);
    await d.call('genie_poll_response', first);
    const follow = await ask(d, 'follow-up', first.conversation_id);
    expect(follow.isError).toBe(false);
    const ref = ids(follow);
    expect(ref.conversation_id).toBe(first.conversation_id);
    expect(ref.response_id).not.toBe(first.response_id);
  });

  it('[AT-22] unknown status: the double keeps returning unknown, then completes with the same IDs', async () => {
    const d = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: WSID,
      responses: [scenarios.unknownStatus(3)],
    });
    const ref = ids(await ask(d, 'q'));
    const statuses: string[] = [];
    for (let i = 0; i < 4; i++) {
      const p = await d.call('genie_poll_response', ref);
      statuses.push((p.structuredContent as { status: string }).status);
    }
    expect(statuses).toEqual(['unknown', 'unknown', 'unknown', 'completed']);
    expect(d.polls().every((c) => c.args.response_id === ref.response_id)).toBe(true);
    expect(d.asks()).toHaveLength(1);
  });

  it('[AT-22] null deep_link: the terminal poll has deep_link null', async () => {
    const d = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: WSID,
      responses: [scenarios.nullDeepLink()],
    });
    const ref = ids(await ask(d, 'q'));
    const p = await d.call('genie_poll_response', ref);
    expect(p.structuredContent).toMatchObject({ status: 'completed', deep_link: null });
  });

  it('[AT-22] null progress_steps: in-progress polls have progress_steps null', async () => {
    const d = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: WSID,
      responses: [scenarios.nullProgressSteps()],
    });
    const ref = ids(await ask(d, 'q'));
    const p = await d.call('genie_poll_response', ref);
    expect(p.structuredContent).toMatchObject({ status: 'in_progress', progress_steps: null });
  });

  it('[AT-22] records the peak number of overlapping polls on one conversation', async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const d = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: WSID,
      responses: [scenarios.normalAnswer()],
      beforeReply: (tool) => (tool === 'genie_poll_response' ? gate : undefined),
    });
    const ref = ids(await ask(d, 'q'));
    const a = d.call('genie_poll_response', ref);
    const b = d.call('genie_poll_response', ref);
    release();
    await Promise.all([a, b]);
    expect(d.maxConcurrentPolls()).toBe(2);
  });

  it('[AT-22] serial polls keep the overlap peak at 1', async () => {
    const d = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: WSID,
      responses: [scenarios.normalAnswer()],
    });
    const ref = ids(await ask(d, 'q'));
    await d.call('genie_poll_response', ref);
    await d.call('genie_poll_response', ref);
    expect(d.maxConcurrentPolls()).toBe(1);
  });

  it('[AT-22] polling an unknown response is an isError result', async () => {
    const d = createGenieOneMcpDouble({ host: HOST, workspaceId: WSID });
    const p = await d.call('genie_poll_response', { conversation_id: 'c', response_id: 'r' });
    expect(p.isError).toBe(true);
  });

  it('[AT-22] cancel frees the conversation and the cancelled response polls as failed', async () => {
    const d = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: WSID,
      responses: [scenarios.unknownStatus(10), scenarios.normalAnswer()],
    });
    const ref = ids(await ask(d, 'q'));
    const c = await d.call('genie_cancel_response', { conversation_id: ref.conversation_id });
    expect(c.isError).toBe(false);
    const p = await d.call('genie_poll_response', ref);
    expect(p.structuredContent).toMatchObject({ status: 'failed' });
    const again = await ask(d, 'q2', ref.conversation_id);
    expect(again.isError).toBe(false);
  });

  it('[AT-22] returns a scripted query result and an error for an unknown item', async () => {
    const d = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: WSID,
      responses: [scenarios.normalAnswer()],
    });
    const ref = ids(await ask(d, 'q'));
    const ok = await d.call('genie_get_query_result', { ...ref, item_id: 'q1' });
    expect(ok.isError).toBe(false);
    expect(ok.structuredContent).toMatchObject({
      columns: [{ name: 'revenue' }],
      rows: [[1200000]],
    });
    const bad = await d.call('genie_get_query_result', { ...ref, item_id: 'nope' });
    expect(bad.isError).toBe(true);
  });

  it('[AT-22] the double never sees a token: requests carry only tool arguments', async () => {
    const d = createGenieOneMcpDouble({
      host: HOST,
      workspaceId: WSID,
      responses: [scenarios.normalAnswer()],
    });
    await ask(d, 'q');
    expect(JSON.stringify(d.calls())).not.toMatch(/token|bearer|dapi/i);
  });
});
