/**
 * The chip row of the card (UX-20a, UX-20b, UX-20c) and the chip catalogue (Appendix E, ADR 0043).
 *
 * - UX-20a: the row shows 4 chips or less. A 5th chip, and later ones, go to the expanded card.
 * - UX-20b: each chip has a text label. A label with no letter or digit (an icon only) is refused.
 * - UX-20c: a chip acts only on a click (`click()`), never on an offer, a hover or an expiry.
 *   Under B1 nothing goes to Genie without that click.
 *
 * The class is pure and has no timers: the caller gives it the monotonic time, like `BlurbStack`.
 * Expiry follows the "Expiry" column of Appendix E. A chip that lives "with the blurb", "with the
 * request" or "while the condition lasts" has no time limit and goes by `remove` or `removeOwner`.
 */

/** Chips in the row of the card. UX-20a. */
export const MAX_CARD_CHIPS = 4;

const S = 1000;
const MIN = 60 * S;
const HOUR = 60 * MIN;

export interface ChipDef {
  /** The expiry time in ms after the offer. */
  ttlMs?: number;
  /** The chip expires when this many newer chips are offered. */
  newer?: number;
  /** The chip expires when the user opens it. */
  onOpen?: boolean;
  /** The action sends data out (Genie or the agent). It runs only on a click (B1). */
  sends?: boolean;
  /** A v1 feature: refused in the MVP. */
  v1?: boolean;
  /** A badge, not a chip (ADR 0029): the chip row refuses it. */
  badge?: boolean;
}

/** The 24 rows of the Appendix E chip catalogue, in the order of its numbers. */
const DEFS = {
  ask_genie: { ttlMs: 2 * MIN, newer: 3, sends: true },
  ask_genie_anyway: { sends: true },
  context_included: {},
  resend_without_context: { sends: true },
  saved_to_memory: { ttlMs: 8 * S },
  synced_to_home: { ttlMs: 8 * S, v1: true },
  genie_finished: { ttlMs: 24 * HOUR, onOpen: true },
  genie_busy: {},
  sign_in: {},
  show_admin_checklist: {},
  keep_recording: {},
  still_recording: {},
  meeting_may_have_ended: { ttlMs: 10 * MIN, v1: true },
  switch_profile: { ttlMs: 30 * S },
  choose_profile: {},
  clarify_answer: { sends: true },
  new_topic: {},
  send_summary: { ttlMs: 24 * HOUR, sends: true },
  check_again: { sends: true },
  sync_paused: { badge: true, v1: true },
  stt_badge: { badge: true },
  copy: { ttlMs: 30 * S },
  draft_page: { ttlMs: 24 * HOUR },
  select_send: { sends: true },
} as const satisfies Record<string, ChipDef>;

export type ChipKind = keyof typeof DEFS;

export const CHIP_CATALOGUE: Record<ChipKind, ChipDef> = DEFS;

export interface ChipInput {
  id: string;
  kind: ChipKind;
  /** The visible text (UX-20b). */
  label: string;
  /** The blurb or request this chip lives with. `removeOwner` takes the chip away. */
  owner?: string;
}

export interface ChipView {
  id: string;
  label: string;
}

/** What the brain runs after a click on a chip (UX-20c). */
export interface ChipAction {
  id: string;
  kind: ChipKind;
}

interface Entry extends ChipInput {
  seq: number;
  expiresAt?: number;
}

/** A label needs a letter or a digit: an emoji or a symbol alone is an icon (UX-20b). */
const hasText = (label: string) => /[\p{L}\p{N}]/u.test(label);

export class ChipRow {
  private entries: Entry[] = [];
  private seq = 0;
  private readonly v1: boolean;

  constructor(opts: { v1?: boolean } = {}) {
    this.v1 = opts.v1 === true;
  }

  /** Offer a chip. Returns false when the row refuses it. A repeated ID keeps its place. */
  offer(c: ChipInput, now: number): boolean {
    const def = CHIP_CATALOGUE[c.kind];
    if (def.badge || (def.v1 && !this.v1) || !hasText(c.label)) return false;
    const at = this.entries.findIndex((e) => e.id === c.id);
    // The same ID again keeps its sequence number: it is not a newer chip (Ask Genie rule).
    const entry: Entry = {
      ...c,
      seq: this.entries[at]?.seq ?? ++this.seq,
      ...(def.ttlMs !== undefined ? { expiresAt: now + def.ttlMs } : {}),
    };
    if (at >= 0) this.entries[at] = entry;
    else {
      this.entries.push(entry);
      this.expireNewer();
    }
    return true;
  }

  /** The "3 newer chips" rule of Ask Genie. Chips with no `newer` limit are untouched. */
  private expireNewer() {
    const latest = this.seq;
    this.entries = this.entries.filter((e) => {
      const n = CHIP_CATALOGUE[e.kind].newer;
      return n === undefined || latest - e.seq < n;
    });
  }

  remove(id: string): boolean {
    const n = this.entries.length;
    this.entries = this.entries.filter((e) => e.id !== id);
    return this.entries.length !== n;
  }

  /** Remove the chips of a blurb or request that ended. Returns the IDs removed. */
  removeOwner(owner: string): string[] {
    const gone = this.entries.filter((e) => e.owner === owner).map((e) => e.id);
    this.entries = this.entries.filter((e) => e.owner !== owner);
    return gone;
  }

  /** Remove the chips that are due at `now`. Returns their IDs. */
  tick(now: number): string[] {
    const gone = this.entries.filter((e) => e.expiresAt !== undefined && e.expiresAt <= now);
    this.entries = this.entries.filter((e) => !gone.includes(e));
    return gone.map((e) => e.id);
  }

  /** The time of the next expiry, or undefined when no chip has a time limit. */
  nextDeadline(): number | undefined {
    const times = this.entries.flatMap((e) => (e.expiresAt === undefined ? [] : [e.expiresAt]));
    return times.length ? Math.min(...times) : undefined;
  }

  /** A click on a chip (UX-20c). The only way to get an action. An unknown ID gives none. */
  click(id: string): ChipAction | undefined {
    const e = this.entries.find((x) => x.id === id);
    if (!e) return undefined;
    if (CHIP_CATALOGUE[e.kind].onOpen) this.remove(id);
    return { id: e.id, kind: e.kind };
  }

  /** The row (4 chips or less, in the order offered) and the chips for the expanded card. */
  view(): { chips: ChipView[]; overflow: ChipView[] } {
    const all = this.entries.map(({ id, label }) => ({ id, label }));
    return { chips: all.slice(0, MAX_CARD_CHIPS), overflow: all.slice(MAX_CARD_CHIPS) };
  }
}
