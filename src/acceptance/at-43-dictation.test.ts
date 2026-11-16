/**
 * AT-43 "UC1 dictation into the active app" (#231), section 15: the brain part. Pass: the text
 * appears with no Enter and the earlier clipboard returns; a password field shows the reason and
 * Copy; without Accessibility the MOG shows Copy; no agent turn and no Genie call; Shift forces
 * the ask route; key-up to inserted text 1.5 s or less at p95 (local).
 *
 * The dictation route (#59) and the dictation card with Copy and the "Inserted" pill (#60) are
 * not built, so these tests stay `it.skip` with the feature issue. The `insert_text` contract
 * (MOG-06b) runs in `crates/mog-core/tests/at_43_dictation.rs`, the paste itself is #58, and the
 * latency row NFR-01d is the #171 benchmark (`crates/mog-core/tests/latency_bench.rs`). The four
 * insertion targets are a person's step (docs/tests/AT-DICTATION-STT-OFFLINE.md).
 */
import { createRouterContextGate } from '../brain/components/router-context-gate';
import { core } from '../brain/test-core';
import { callsTo } from '../test-support/cloud-calls';

vi.mock('@tauri-apps/api/core', async () => (await import('../brain/test-core')).coreModule);

const MOG = '00000000-0000-4000-8000-0000000000a1';

describe('AT-43 UC1 dictation into the active app', () => {
  beforeEach(() => core.reset());

  it.skip('[AT-43] [RTR-01b] [RTR-01c] a plain sentence is dictation and ends at insert_text with no agent turn and no Genie call TODO(#59)', async () => {
    const d = await createRouterContextGate().route(
      'Hi Sam, the deck is attached and I will send the numbers tomorrow.',
      MOG,
    );
    expect(d.intent).toBe('dictation');
    // TODO(#59): run the UC1 flow from key-up with this decision.
    expect(callsTo(core.invoke.mock.calls, ['insert_text'])).toEqual(['insert_text']);
    expect(callsTo(core.invoke.mock.calls, ['agent_send', 'agent_start', 'mcp_call'])).toEqual([]);
  });

  it.skip('[AT-43] [RTR-17a] Shift held with the talk key forces the ask route TODO(#59)', () => {
    // TODO(#59): the same plain sentence with Shift held goes to the ask route (preview chip),
    // not to insert_text; the forced route skips the intent gate only.
    expect(callsTo(core.invoke.mock.calls, ['insert_text'])).toEqual([]);
    expect.unreachable('TODO(#59): the Shift-forced route is not built');
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
