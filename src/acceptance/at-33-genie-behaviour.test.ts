/**
 * AT-33 "Live Genie One MCP behaviours" (#234), section 15 (live). Pass: busy text →
 * CONVERSATION_BUSY with no retry; null steps show elapsed time; unknown keeps polling; the
 * fallback link is correct; query items merge.
 *
 * The card link rule ships (SEC-25a, #210): `trustedUrl` passes only an https link on the
 * profile host. That is the G-19b rule, and it runs here on the deep links of the #229 Genie
 * One MCP double. The double's busy, unknown and null cases are its own `[AT-22]` self-tests;
 * the GL-5 error classes and the busy rule run here (#32, ADR 0072). Still pending: the
 * progress line, the fallback link and the query item merge (#87), and the unknown re-poll
 * (#31). SP-09 (#29, `spikes/sp-09`) proves the busy mapping on its own client offline. The live
 * run is a person step (docs/tests/AT-GENIE-MCP.md).
 */
import { trustedUrl } from '../views/card/safe-markdown';
import { createGenieBridge } from '../brain/components/genie-bridge';
import { sendGenieAsk } from '../brain/components/genie-errors';
import { BUSY_TEXT, createGenieOneMcpDouble, scenarios } from '../test-support/genie-one-mcp';
import { liveFixture } from '../test-support/fixtures';

const HOST = 'example.cloud.databricks.com';
const WSID = '1234567890';
const trust = { profileHost: HOST, allowedAppUrls: [] };

async function deepLink() {
  const d = createGenieOneMcpDouble({
    host: HOST,
    workspaceId: WSID,
    responses: [scenarios.normalAnswer()],
  });
  const a = (await d.call('genie_ask', { question: 'q' })).structuredContent as {
    conversation_id: string;
    response_id: string;
  };
  const ref = { conversation_id: a.conversation_id, response_id: a.response_id };
  await d.call('genie_poll_response', ref);
  const done = (await d.call('genie_poll_response', ref)).structuredContent as {
    deep_link: string;
  };
  return { link: done.deep_link, cid: a.conversation_id };
}

describe('AT-33 live Genie One MCP behaviours', () => {
  it('[AT-33] [G-19b] the deep link on the profile host shows', async () => {
    const { link, cid } = await deepLink();
    expect(link).toBe(
      `https://${HOST}/one/chat/threads/${cid}?o=${WSID}&utm_source=genie_chat_mcp`,
    );
    expect(trustedUrl(link, trust)).toBe(link);
  });

  it('[AT-33] [G-19b] a link to another host or over http does not show', async () => {
    const { link } = await deepLink();
    expect(trustedUrl(link.replace(HOST, 'evil.example.test'), trust)).toBeUndefined();
    expect(trustedUrl(link.replace(HOST, `${HOST}.evil.example.test`), trust)).toBeUndefined();
    expect(trustedUrl(link.replace('https://', 'http://'), trust)).toBeUndefined();
    expect(
      trustedUrl(link.replace(`https://${HOST}`, `https://u:p@${HOST}`), trust),
    ).toBeUndefined();
  });

  it('[AT-33] [G-14] [G-15a] [G-15b] the busy text maps to CONVERSATION_BUSY and nothing is sent again', async () => {
    expect(createGenieBridge().mapError({ isErrorText: BUSY_TEXT }).code).toBe('CONVERSATION_BUSY');
    for (const busyEnvelope of ['isError', 'jsonrpc'] as const) {
      const d = createGenieOneMcpDouble({
        host: HOST,
        workspaceId: WSID,
        busyEnvelope,
        responses: [scenarios.normalAnswer()],
      });
      const first = await sendGenieAsk(d.call, { question: 'q1' }, { host: HOST });
      if (first.t !== 'accepted') throw new Error('the first ask must be accepted');
      const busy = await sendGenieAsk(
        d.call,
        { question: 'q2', conversation_id: first.conversationId },
        { host: HOST },
      );
      expect(busy).toMatchObject({ t: 'failed', code: 'CONVERSATION_BUSY', retry: 'user' });
      // One genie_ask reached the busy conversation, and nothing was sent again.
      const onBusy = d
        .calls()
        .filter((c) => c.tool === 'genie_ask' && c.args.conversation_id === first.conversationId);
      expect(onBusy).toHaveLength(1);
      expect(d.calls().filter((c) => c.tool === 'genie_ask')).toHaveLength(2);
    }
  });

  it.skip('[AT-33] [G-17a] [G-17b] null progress steps show the elapsed time; a step replaces the older line TODO(#87)', () => {
    // When #87 lands: `scenarios.nullProgressSteps()` gives "Working… 0:12" at 12 s on the sim
    // clock; `scenarios.normalAnswer()` shows "Reading the tables".
    expect.unreachable('#87: the progress line is not built');
  });

  it.skip('[AT-33] [G-19a] a null deep_link falls back to /one/chat/threads/<cid>?o=<wsid> TODO(#87)', () => {
    // When #87 lands: `scenarios.nullDeepLink()` gives answer.deepLink
    // https://<host>/one/chat/threads/<cid>?o=<wsid>.
    expect.unreachable('#87: the fallback link is not built');
  });

  it.skip('[AT-33] [G-20a] [G-20b] query items merge across polls; only pub_*_output IDs stay TODO(#87)', () => {
    expect.unreachable('#87: the query item merge is not built');
  });

  it.skip('[AT-33] [G-18a] [G-18b] unknown status keeps polling with the same IDs until the give-up TODO(#31)', () => {
    // When #31 lands: `scenarios.unknownStatus(n)` gets n + 1 polls with one ask and no re-ask.
    expect.unreachable('#31: the PollActor is not built');
  });

  it.skip('[AT-33] live: a second ask to a busy conversation and a short question TODO(#29)', (ctx) => {
    const live = liveFixture();
    if (!live.enabled) return ctx.skip(live.reason);
    // The GL-5 classes are built (#32). The live run (SP-09, #29, docs/tests/AT-GENIE-MCP.md)
    // records the busy envelope and proves that the live busy reply maps the same way.
    expect.unreachable('#29: the live Genie One MCP run is a person step');
  });
});
