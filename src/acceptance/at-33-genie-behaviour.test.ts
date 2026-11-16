/**
 * AT-33 "Live Genie One MCP behaviours" (#234), section 15 (live). Pass: busy text →
 * CONVERSATION_BUSY with no retry; null steps show elapsed time; unknown keeps polling; the
 * fallback link is correct; query items merge.
 *
 * The card link rule ships (SEC-25a, #210): `trustedUrl` passes only an https link on the
 * profile host. That is the G-19b rule, and it runs here on the deep links of the #229 Genie
 * One MCP double. The double's busy, unknown and null cases are its own `[AT-22]` self-tests;
 * the bridge rules on top of them are pending: the GL-5 error classes and the busy rule (#32),
 * the progress line, the fallback link and the query item merge (#87), and the unknown re-poll
 * (#31). SP-09 (#29, `spikes/sp-09`) proves the busy mapping on its own client offline. The live
 * run is a person step (docs/tests/AT-GENIE-MCP.md).
 */
import { trustedUrl } from '../views/card/safe-markdown';
import { createGenieBridge } from '../brain/components/genie-bridge';
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

  it.skip('[AT-33] [G-14] [G-15a] [G-15b] the busy text maps to CONVERSATION_BUSY and nothing is sent again TODO(#32)', () => {
    const r = createGenieBridge().mapError({ isErrorText: BUSY_TEXT });
    expect(r.code).toBe('CONVERSATION_BUSY');
    // When #32 lands, also through submit: the double gets one genie_ask on the busy
    // conversation and no second one without a user action.
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

  it.skip('[AT-33] live: a second ask to a busy conversation and a short question TODO(#32)', (ctx) => {
    const live = liveFixture();
    if (!live.enabled) return ctx.skip(live.reason);
    expect.unreachable('#32: the live path needs the error classes');
  });
});
