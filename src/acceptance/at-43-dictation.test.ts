/**
 * AT-43 "UC1 dictation into the active app" (#231), section 15: the brain part. Pass: the text
 * appears with no Enter and the earlier clipboard returns; a password field shows the reason and
 * Copy; without Accessibility the MOG shows Copy; no agent turn and no Genie call; ⌥G forces
 * the ask route; key-up to inserted text 1.5 s or less at p95 (local).
 *
 * The router step ships (#57), so its part of RTR-01b and RTR-01c runs here with a model reply
 * from the IPC double: a dictation decision stays dictation for PTT and hands-free, and the step
 * calls only the router model, never the agent, Genie or `insert_text`. Whether a real model
 * gives `dictation` for plain sentences is the live router golden set (#56, AT-06). The route to
 * insertion and the ⌥G-forced ask route (#59, ADR 0098) run through `routeUtterance`. The dictation card
 * with Copy and the "Inserted" pill (#60) are not built, so those tests stay `it.skip` with the
 * feature issue. The `insert_text` contract (MOG-06b) and the paste (#58) run in
 * `crates/mog-core/tests/at_43_dictation.rs`, and the
 * latency row NFR-01d is the #171 benchmark (`crates/mog-core/tests/latency_bench.rs`). The four
 * insertion targets are a person's step (docs/tests/AT-DICTATION-STT-OFFLINE.md).
 */
import type { CaptureMode } from '../shared/interfaces';
import { createRouterContextGate } from '../brain/components/router-context-gate';
import type { RouterInput } from '../brain/components/router-step';
import { core } from '../brain/test-core';
import { ipcUtteranceDeps, routeUtterance, type Utterance } from '../brain/utterance-route';
import { argsOf, callsTo } from '../test-support/cloud-calls';
import { chatCompletion, routerDecision } from '../test-support/model-invoke';

vi.mock('@tauri-apps/api/core', async () => (await import('../brain/test-core')).coreModule);

const MOG = '00000000-0000-4000-8000-0000000000a1';
const SENTENCE = 'Hi Sam, the deck is attached and I will send the numbers tomorrow.';
const input = (captureMode: CaptureMode): RouterInput => ({
  profile: 'caller-profile',
  speakerTrack: 'you',
  captureMode,
  pendingClarification: '',
  recentTopics: [],
  activeMogs: [{ id: MOG, name: 'Finance' }],
  profiles: [{ name: 'home', host: 'https://home.example.test', aliases: [] }],
  currentTarget: 'home',
});
const utterance = (forceAsk: boolean): Utterance => ({
  mogId: MOG,
  text: SENTENCE,
  leaseId: 'lease-1',
  forceAsk,
  insertionOn: true,
  input: input('ptt'),
});
const dictationReply = () =>
  core.invoke.mockImplementation(async (cmd: string) =>
    cmd === 'model_invoke'
      ? chatCompletion(
          JSON.stringify(
            routerDecision({
              intent: 'dictation',
              cleaned_text: SENTENCE,
              genie_question: '',
              topic_label: '',
            }),
          ),
        )
      : cmd === 'insert_text'
        ? { result: 'inserted' }
        : {},
  );

describe('AT-43 UC1 dictation into the active app', () => {
  beforeEach(() => core.reset());

  it('[AT-43] [RTR-01b] the router step keeps a dictation decision for PTT and hands-free only', async () => {
    dictationReply();
    const gate = createRouterContextGate();
    expect((await gate.route(SENTENCE, MOG, input('ptt'))).intent).toBe('dictation');
    expect((await gate.route(SENTENCE, MOG, input('hands_free'))).intent).toBe('dictation');
    // A meeting utterance is never inserted into the focused app.
    expect((await gate.route(SENTENCE, MOG, input('session'))).intent).not.toBe('dictation');
  });

  it('[AT-43] [RTR-01c] the router step for a dictation calls only the router model', async () => {
    dictationReply();
    await createRouterContextGate().route(SENTENCE, MOG, input('ptt'));
    const log = core.invoke.mock.calls;
    expect(log.map(([cmd]) => cmd)).toEqual(['model_invoke']);
    expect((argsOf(log[0]?.[1]) as { role?: string } | undefined)?.role).toBe('router');
    expect(
      callsTo(log, ['agent_start', 'agent_send', 'agent_tool_result', 'mcp_call', 'insert_text']),
    ).toEqual([]);
  });

  it('[AT-43] [RTR-01b] [RTR-01c] a dictation ends at insert_text with no agent turn and no Genie call', async () => {
    dictationReply();
    const r = await routeUtterance(utterance(false), ipcUtteranceDeps());
    expect(r).toMatchObject({ route: 'chip', dictated: { route: 'inserted' } });
    const log = core.invoke.mock.calls;
    expect(log.map(([cmd]) => cmd)).toEqual(['model_invoke', 'insert_text']);
    expect(argsOf(log[1]?.[1])).toEqual({ mogId: MOG, text: SENTENCE, leaseId: 'lease-1' });
    expect(callsTo(log, ['agent_start', 'agent_send', 'agent_tool_result', 'mcp_call'])).toEqual(
      [],
    );
  });

  it('[AT-43] [RTR-17a] ⌥G during the talk-key hold forces the ask route with the exact text', async () => {
    dictationReply();
    const r = await routeUtterance(utterance(true), ipcUtteranceDeps());
    // The same plain sentence goes to the ask route, not to insert_text, and with no preview.
    expect(r).toMatchObject({ route: 'ask', question: SENTENCE });
    expect(callsTo(core.invoke.mock.calls, ['insert_text'])).toEqual([]);
    // The route itself sends nothing: only the T3 ask gate calls the send seam.
    expect(callsTo(core.invoke.mock.calls, ['agent_start', 'agent_send', 'mcp_call'])).toEqual([]);
  });

  it.skip('[AT-43] [MOG-06d] a failed insertion shows the cleaned text, Copy and the reason TODO(#60)', () => {
    // TODO(#60): for each insert_text result no_permission, no_focus and secure_field, the card
    // shows the cleaned text, a Copy button and the reason text (the password field and the run
    // without Accessibility of the AT script).
    for (const result of ['no_permission', 'no_focus', 'secure_field'] as const) {
      core.invoke.mockResolvedValueOnce({ result });
    }
    expect.unreachable('TODO(#60): the dictation card is not built');
  });

  it.skip('[AT-43] [UX-42] the pill shows "Inserted" for 1.2 s after each insertion TODO(#60)', () => {
    // TODO(#60): on the sim clock, after insert_text gives `inserted`, the mood publisher sends
    // the pill "Inserted"; it is gone 1.2 s later.
    expect.unreachable('TODO(#60): the "Inserted" pill is not built');
  });
});
