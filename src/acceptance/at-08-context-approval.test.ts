/**
 * AT-08 "Approve exact context" (#234), section 15 (live). Pass: the exact payload and target
 * show; each change cancels the approval; caps hold; one retry without preamble occurs; the
 * audit hash matches.
 *
 * Only the display part of G-35a and G-35b ships (#89): the expanded card shows the "Sent to
 * Genie" text exactly as the brain gives it, and offers "Resend without context" when a preamble
 * went. That runs here. The ContextGate approval binding and the POLICY_DENIED retry are #152,
 * the preamble format and once-per-item rules #92, the size caps #90, the item filter and PII
 * scrub #91, and the content-free audit entry #140. Those tests are pending with the contract
 * shapes of `src/shared/interfaces.ts`. The live run is a person step (docs/tests/AT-GENIE-MCP.md).
 */
import type { AskRequest, ContextPreamble } from '../shared/interfaces';
import { buildSections } from '../brain/card-sections';
import { createRouterContextGate } from '../brain/components/router-context-gate';
import { liveFixture } from '../test-support/fixtures';

const MOG = '00000000-0000-4000-8000-0000000000c8';
const HEADER: ContextPreamble['header'] = 'Context (from my notes, may be wrong):';
const preamble: ContextPreamble = {
  header: HEADER,
  items: [{ id: 'm1', text: 'Q3 means the fiscal quarter.', source: 'you', ts: 1, truncated: false }],
  chars: 28,
};
const req = (over: Partial<AskRequest> = {}): AskRequest => ({
  opId: 'op-1',
  mogId: MOG,
  binding: { kind: 'genie_one_mcp', profile: 'bottle-test' },
  question: 'What was revenue in Q3?',
  preamble,
  conversationId: 'conv-1',
  tags: { app: 'bottle', mog: MOG, op: 'genie' },
  ...over,
});
const sent = `${HEADER}\n- ${preamble.items[0]?.text}\n\nWhat was revenue in Q3?`;

describe('AT-08 approve exact context', () => {
  it('[AT-08] [G-35a] the expanded card shows the exact sent text, unchanged', () => {
    const s = buildSections({ sentToGenie: sent, canResend: true, dataFlowText: 'x' });
    const shown = s.find((x) => x.id === 'sentToGenie');
    expect(shown).toEqual({ id: 'sentToGenie', text: sent });
  });

  it('[AT-08] [G-35b] "Resend without context" shows only when a preamble went', () => {
    const withCtx = buildSections({ sentToGenie: sent, canResend: true, dataFlowText: 'x' });
    expect(withCtx.some((x) => x.id === 'resend')).toBe(true);
    const plain = buildSections({ answerMd: 'Done.', dataFlowText: 'x' });
    expect(plain.some((x) => x.id === 'resend' || x.id === 'sentToGenie')).toBe(false);
  });

  it.skip('[AT-08] [G-34a] [G-34b] a change of payload, target, conversation or attempt cancels the approval TODO(#152)', () => {
    const gate = createRouterContextGate();
    const { approvalId } = gate.approve(req(), 1);
    expect(gate.verify(approvalId, req(), 1)).toBe(true);
    expect(gate.verify(approvalId, req({ question: 'What was revenue in Q4?' }), 1)).toBe(false);
    expect(
      gate.verify(approvalId, req({ binding: { kind: 'genie_one_mcp', profile: 'other' } }), 1),
    ).toBe(false);
    expect(gate.verify(approvalId, req({ conversationId: 'conv-2' }), 1)).toBe(false);
    expect(gate.verify(approvalId, req(), 2)).toBe(false);
  });

  it.skip('[AT-08] [G-33a] [G-33b] POLICY_DENIED sends once without the preamble, never with more TODO(#152)', () => {
    // When #152 lands: the double returns a POLICY_DENIED tool error to the first genie_ask; the
    // bridge sends the question once with no preamble; a second denial shows "A workspace policy
    // blocked this question." and no third ask goes.
    expect.unreachable('#152: the POLICY_DENIED retry is not built');
  });

  it.skip('[AT-08] [G-28] [G-29a] [G-29b] [G-29c] [G-29d] [G-32a] [G-32b] preamble format, caps and once-per-item TODO(#92)', () => {
    // When #92 lands: context off by default sends the question exactly; the header matches;
    // a 6th item, a 161-char item and an 801-char preamble are cut at the rules' edges; a
    // follow-up without new items sends no preamble; an item goes once per conversation.
    expect.unreachable('#92: the ContextAssembler is not built');
  });

  it.skip('[AT-08] [G-30a] [G-30b] [G-30c] question, message and byte caps TODO(#90)', () => {
    // When #90 lands: a 1,001-char question is cut at a word edge with a notice; a message over
    // 2,000 chars drops preamble items first; a message over 25,000 UTF-8 bytes is blocked.
    expect.unreachable('#90: the sanitiser caps are not built');
  });

  it.skip('[AT-08] [G-31] [G-37a] [G-37b] [G-37c] Table G-CTX filter and PII scrub TODO(#91)', () => {
    expect.unreachable('#91: the preamble filter and PII scrub are not built');
  });

  it.skip('[AT-08] [SEC-23] the audit entry holds IDs, sizes and the hash of the sent text only TODO(#140)', () => {
    // When #140 lands: the hash in the audit entry equals the hash of the "Sent to Genie" text
    // (G-35a), and the entry has no question, preamble or answer text.
    expect.unreachable('#140: the audit log is not built');
  });

  it.skip('[AT-08] live: approve a bounded payload and force POLICY_DENIED TODO(#152)', (ctx) => {
    const live = liveFixture();
    if (!live.enabled) return ctx.skip(live.reason);
    expect.unreachable('#152: the live path needs the ContextGate');
  });
});
