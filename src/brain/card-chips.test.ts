import { CHIP_CATALOGUE, MAX_CARD_CHIPS, ChipRow, type ChipInput } from './card-chips';

const S = 1000;
const chip = (id: string, kind: ChipInput['kind'], label = 'Label', owner?: string): ChipInput => ({
  id,
  kind,
  label,
  ...(owner ? { owner } : {}),
});

describe('card chip row', () => {
  it('[UX-20a] [AT-20] shows 4 chips or less and moves the 5th and later chips to the expanded card', () => {
    const row = new ChipRow();
    for (let i = 1; i <= 6; i++) row.offer(chip(`c${i}`, 'show_admin_checklist', `Chip ${i}`), 0);
    const v = row.view();
    expect(MAX_CARD_CHIPS).toBe(4);
    expect(v.chips.map((c) => c.id)).toEqual(['c1', 'c2', 'c3', 'c4']);
    expect(v.overflow.map((c) => c.id)).toEqual(['c5', 'c6']);
  });

  it('[UX-20a] a chip that expires or goes lets an overflow chip take its place', () => {
    const row = new ChipRow();
    for (let i = 1; i <= 5; i++) row.offer(chip(`c${i}`, 'show_admin_checklist', `Chip ${i}`), 0);
    row.remove('c2');
    expect(row.view().chips.map((c) => c.id)).toEqual(['c1', 'c3', 'c4', 'c5']);
    expect(row.view().overflow).toEqual([]);
  });

  it('[UX-20b] [AT-20] refuses a chip with no text label, or an icon-only label', () => {
    const row = new ChipRow();
    expect(row.offer(chip('a', 'ask_genie', ''), 0)).toBe(false);
    expect(row.offer(chip('b', 'ask_genie', '   '), 0)).toBe(false);
    expect(row.offer(chip('c', 'ask_genie', '✨'), 0)).toBe(false);
    expect(row.offer(chip('d', 'ask_genie', '✨ Ask Genie'), 0)).toBe(true);
    expect(row.view().chips.map((c) => c.id)).toEqual(['d']);
  });

  it('[UX-20c] [AT-20] offering and expiry call no action; only click does, once per click', () => {
    const row = new ChipRow();
    row.offer(chip('a', 'ask_genie', 'Ask Genie'), 0);
    row.view();
    row.tick(10 * S);
    expect(row.click('nope')).toBeUndefined();
    expect(row.click('a')).toEqual({ id: 'a', kind: 'ask_genie' });
  });

  it('[UX-20c] only the catalogue marks the chips that send data, and each needs a click', () => {
    const sending = Object.entries(CHIP_CATALOGUE)
      .filter(([, d]) => d.sends)
      .map(([k]) => k)
      .sort();
    expect(sending).toEqual(
      [
        'ask_genie',
        'ask_genie_anyway',
        'check_again',
        'clarify_answer',
        'resend_without_context',
        'select_send',
        'send_summary',
      ].sort(),
    );
  });

  it('[UX-20a] the catalogue has the 24 chips of Appendix E', () => {
    expect(Object.keys(CHIP_CATALOGUE)).toHaveLength(24);
  });

  it('[UX-20a] badges are not chips: the STT badge and the sync paused badge are refused', () => {
    const row = new ChipRow({ v1: true });
    expect(row.offer(chip('s', 'stt_badge', 'STT: local'), 0)).toBe(false);
    expect(row.offer(chip('p', 'sync_paused', 'Sync paused'), 0)).toBe(false);
  });

  it('[UX-20a] v1 chips (synced, meeting may have ended) show only when v1 is on', () => {
    const mvp = new ChipRow();
    expect(mvp.offer(chip('a', 'synced_to_home', 'Synced to host'), 0)).toBe(false);
    expect(mvp.offer(chip('b', 'meeting_may_have_ended', 'Meeting may have ended. Stop?'), 0)).toBe(
      false,
    );
    const v1 = new ChipRow({ v1: true });
    expect(v1.offer(chip('a', 'synced_to_home', 'Synced to host'), 0)).toBe(true);
  });
});

describe('card chip expiry (Appendix E)', () => {
  it('[UX-20a] Saved to memory expires after 8 s', () => {
    const row = new ChipRow();
    row.offer(chip('m', 'saved_to_memory', 'Saved to memory'), 0);
    expect(row.nextDeadline()).toBe(8 * S);
    expect(row.tick(7999)).toEqual([]);
    expect(row.tick(8 * S)).toEqual(['m']);
    expect(row.view().chips).toEqual([]);
  });

  it.each([
    ['copy', 30 * S],
    ['switch_profile', 30 * S],
    ['meeting_may_have_ended', 10 * 60 * S],
    ['send_summary', 24 * 3600 * S],
    ['draft_page', 24 * 3600 * S],
    ['genie_finished', 24 * 3600 * S],
  ] as const)('[UX-20a] %s expires on its time', (kind, ms) => {
    const row = new ChipRow({ v1: true });
    row.offer(chip('x', kind, 'Some label'), 100);
    expect(row.nextDeadline()).toBe(100 + ms);
    expect(row.tick(100 + ms - 1)).toEqual([]);
    expect(row.tick(100 + ms)).toEqual(['x']);
  });

  it('[UX-20a] Ask Genie expires after 2 min', () => {
    const row = new ChipRow();
    row.offer(chip('q', 'ask_genie', 'Ask Genie'), 0);
    expect(row.tick(120 * S - 1)).toEqual([]);
    expect(row.tick(120 * S)).toEqual(['q']);
  });

  it('[UX-20a] Ask Genie expires after 3 newer chips', () => {
    const row = new ChipRow();
    row.offer(chip('q', 'ask_genie', 'Ask Genie'), 0);
    row.offer(chip('n1', 'saved_to_memory', 'Saved to memory'), 1);
    row.offer(chip('n2', 'show_admin_checklist', 'Show admin checklist'), 2);
    expect(row.view().chips.map((c) => c.id)).toContain('q');
    row.offer(chip('n3', 'show_admin_checklist', 'Show admin checklist 2'), 3);
    expect(row.view().chips.map((c) => c.id)).not.toContain('q');
  });

  it('[UX-20a] Genie finished expires when it is opened', () => {
    const row = new ChipRow();
    row.offer(chip('g', 'genie_finished', 'Genie finished: Revenue'), 0);
    expect(row.click('g')).toEqual({ id: 'g', kind: 'genie_finished' });
    expect(row.view().chips).toEqual([]);
  });

  it('[UX-20a] a chip that lives with its blurb or request goes with its owner', () => {
    const row = new ChipRow();
    row.offer(chip('r', 'resend_without_context', 'Resend without context', 'b1'), 0);
    row.offer(chip('c', 'context_included', 'context included (12 chars)', 'b1'), 0);
    row.offer(chip('k', 'ask_genie_anyway', 'Ask Genie anyway', 'b2'), 0);
    expect(row.nextDeadline()).toBeUndefined();
    expect(row.removeOwner('b1').sort()).toEqual(['c', 'r']);
    expect(row.view().chips.map((c) => c.id)).toEqual(['k']);
  });

  it('[UX-20a] a chip with no time limit stays until its condition ends (Sign in)', () => {
    const row = new ChipRow();
    row.offer(chip('s', 'sign_in', 'Sign in'), 0);
    expect(row.tick(10 ** 12)).toEqual([]);
    row.remove('s');
    expect(row.view().chips).toEqual([]);
  });

  it('[UX-20a] offering the same chip again keeps its place and restarts its time', () => {
    const row = new ChipRow();
    row.offer(chip('m', 'saved_to_memory', 'Saved to memory'), 0);
    row.offer(chip('o', 'sign_in', 'Sign in'), 1);
    row.offer(chip('m', 'saved_to_memory', 'Saved to memory'), 5 * S);
    expect(row.view().chips.map((c) => c.id)).toEqual(['m', 'o']);
    expect(row.nextDeadline()).toBe(13 * S);
  });
});
