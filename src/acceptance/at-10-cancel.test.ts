/**
 * AT-10 "Cancel with uncertain result" (#234), section 15 (live). Pass: the UI says
 * "Cancellation requested"; polling stops; no remote-success claim appears.
 *
 * The card Esc rule ships (UX-19b, #78): Esc during a turn makes exactly one cancel call with the
 * exchange ref, and the card stays so it can show "Cancellation requested". That runs here, with
 * the #229 Genie One MCP double as the remote side. The `genie_cancel_response` call in the test
 * is test glue in the cancel callback; the bridge call itself (G-25a) is #151. The bridge
 * cancel (`genie_cancel_response`, the text, the poll stop and the `cancelled` row) is #151, and
 * the acknowledgement / no effect / unknown transport cases are the OperationTracker (#39).
 * Those are pending. The live run is a person step (docs/tests/AT-GENIE-MCP.md).
 */
import type { ExchangeRef, MogId } from '../shared/interfaces';
import { handleCardEsc } from '../brain/card-esc';
import { createGenieOneMcpDouble, scenarios } from '../test-support/genie-one-mcp';
import { liveFixture } from '../test-support/fixtures';

const HOST = 'example.cloud.databricks.com';
const MOG = '00000000-0000-4000-8000-0000000000d0' as MogId;

async function turnInFlight() {
  const d = createGenieOneMcpDouble({
    host: HOST,
    workspaceId: '1234567890',
    responses: [scenarios.unknownStatus(10)],
  });
  const a = (await d.call('genie_ask', { question: 'q' })).structuredContent as {
    conversation_id: string;
    response_id: string;
  };
  const ref: ExchangeRef = {
    backend: 'genie_one_mcp',
    profile: 'bottle-test',
    host: HOST,
    conversationId: a.conversation_id,
    responseId: a.response_id,
  };
  return { d, ref };
}

describe('AT-10 cancel with uncertain result', () => {
  it('[AT-10] [UX-19b] Esc during a turn makes one cancel call with the ref and keeps the card', async () => {
    const { d, ref } = await turnInFlight();
    const cancelled: ExchangeRef[] = [];
    const hidden: MogId[] = [];
    const r = await handleCardEsc(
      MOG,
      { inFlight: ref },
      {
        cancel: async (x) => {
          cancelled.push(x);
          // genie-tools.json: the cancel is conversation-scoped and takes no response ID.
          const c = await d.call('genie_cancel_response', { conversation_id: x.conversationId });
          expect(c.structuredContent).toMatchObject({ status: 'cancellation_requested' });
        },
        hideCard: async (m) => void hidden.push(m),
      },
    );
    expect(r).toEqual({ t: 'cancel_request', ok: true });
    expect(cancelled).toEqual([ref]);
    expect(hidden).toEqual([]);
    expect(d.calls().filter((c) => c.tool === 'genie_cancel_response')).toHaveLength(1);
    expect(d.contractViolations()).toEqual([]);
  });

  it('[AT-10] [UX-19b] a cancel whose transport fails does not hide the card either', async () => {
    const { ref } = await turnInFlight();
    const hidden: MogId[] = [];
    const r = await handleCardEsc(
      MOG,
      { inFlight: ref },
      {
        cancel: async () => {
          throw new Error('transport outcome unknown');
        },
        hideCard: async (m) => void hidden.push(m),
      },
    );
    expect(r).toEqual({ t: 'cancel_request', ok: false });
    expect(hidden).toEqual([]);
  });

  it.skip('[AT-10] [G-25a] [G-25b] [G-25c] [G-25d] the bridge cancel shows the text, stops polls and stores cancelled TODO(#151)', () => {
    // When #151 lands: `MogBackend.cancel(ref)` calls genie_cancel_response once; the stream
    // emits cancel_requested; the card shows "Cancellation requested" and never says the remote
    // side stopped; no genie_poll_response follows on the double; the exchange row is cancelled.
    expect.unreachable('#151: the bridge cancel is not built');
  });

  it.skip('[AT-10] [G-25b] acknowledged, no-effect and unknown cancel outcomes never claim remote success TODO(#39)', () => {
    // When #39 lands: script the cancel reply as acknowledged, as no effect (the poll still
    // completes) and as a transport error; the state is CANCEL_REQUESTED or UNCERTAIN, and no
    // text says the remote side stopped.
    expect.unreachable('#39: the OperationTracker is not built');
  });

  it.skip('[AT-10] live: Esc during a turn on a real workspace TODO(#151)', (ctx) => {
    const live = liveFixture();
    if (!live.enabled) return ctx.skip(live.reason);
    expect.unreachable('#151: the live path needs the bridge cancel');
  });
});
