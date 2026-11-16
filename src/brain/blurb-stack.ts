/**
 * The card blurb stack of one MOG (UX-11a, UX-11b, UX-11c, UX-12; ADR 0028).
 *
 * Pure brain logic, with no timers of its own: the caller passes the time (monotonic ms, see
 * `clock.ts`) and arms one timer at `nextDeadline()`. The card view only renders `view()`.
 *
 * - The card shows `maxBlurbs` entries or less (3 or less, UX-11a), newest on top. An older
 *   entry goes to History, and the card shows "N more in History".
 * - An entry is one blurb or one split answer (all parts with one `splitGroupId`). A split takes
 *   one slot (UX-11b).
 * - A pinned entry keeps its slot (UX-11c). At most `maxBlurbs - 1` entries can be pinned, so the
 *   card never shows more than `maxBlurbs` and the newest answer always has a slot.
 * - A completed entry that is not pinned and has no pointer hover or keyboard focus for
 *   `autoHideSec` collapses into History (UX-12). History is one click away.
 *
 * TODO(#103): the hub History view that holds the full blurbs. This stack keeps only the IDs.
 */
import type { ViewEmitKind } from '../shared/ipc';

/** Appendix F: `mog.card.maxBlurbs`. UX-11a caps it at 3. */
export const DEFAULT_MAX_BLURBS = 3;
/** Appendix F: `mog.card.autoHideSec`. */
export const DEFAULT_AUTO_HIDE_SEC = 60;
/** The stack keeps the IDs of this many History entries, newest first. The count has no cap. */
export const HISTORY_IDS_MAX = 100;

export interface StackSettings {
  maxBlurbs: number;
  autoHideSec: number;
}

/** One blurb that the brain puts on the card. TODO(#86): the line templates fill `text`. */
export interface BlurbInput {
  id: string;
  /** Plain text. TODO(#210): safe markdown. */
  text: string;
  /** Parts of one split answer share this ID (GenieExchange.splitGroupId) and take one slot. */
  splitGroupId?: string;
  /** True when the answer is terminal. */
  completed?: boolean;
}

/** One card slot as the view gets it. */
export interface CardBlurb {
  /** The entry ID: the blurb ID, or the `splitGroupId` of a split. */
  id: string;
  text: string;
  pinned: boolean;
  completed: boolean;
  /** The parts of a split, in arrival order. */
  parts?: { id: string; text: string }[];
}

export interface StackView {
  /** Newest first, `maxBlurbs` or less. */
  blurbs: CardBlurb[];
  /** How many entries went to History ("N more in History"). */
  historyCount: number;
}

export type HoverVia = 'pointer' | 'focus';

interface Part {
  id: string;
  text: string;
  completed: boolean;
}

interface Entry {
  id: string;
  split: boolean;
  parts: Part[];
  pinned: boolean;
  hover: Set<HoverVia>;
  /** When the entry last became complete, unhovered and unpinned. */
  quietSince: number;
}

const isCount = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);

function settingsFrom(s: Partial<StackSettings> = {}): StackSettings {
  const max = isCount(s.maxBlurbs)
    ? Math.min(DEFAULT_MAX_BLURBS, Math.max(1, Math.floor(s.maxBlurbs)))
    : DEFAULT_MAX_BLURBS;
  const hide = isCount(s.autoHideSec) && s.autoHideSec > 0 ? s.autoHideSec : DEFAULT_AUTO_HIDE_SEC;
  return { maxBlurbs: max, autoHideSec: hide };
}

export class BlurbStack {
  readonly settings: StackSettings;
  /** Newest first. */
  private entries: Entry[] = [];
  private historyIds: string[] = [];
  private historyTotal = 0;
  /** Every entry ID and blurb ID that went to History. It has no cap, so no blurb comes back. */
  private gone = new Set<string>();

  constructor(settings?: Partial<StackSettings>) {
    this.settings = settingsFrom(settings);
  }

  /**
   * Add a blurb, or update a blurb with the same ID. A split part joins its entry. A blurb or a
   * split part whose ID or group went to History stays there (the store keeps it, G-38a), and
   * a new entry never takes the ID of an entry on the card. Returns false when nothing changed.
   */
  add(b: BlurbInput, now: number): boolean {
    const known = this.findPart(b.id);
    if (known) {
      known.part.text = b.text;
      if (b.completed) this.markComplete(known.entry, known.part, now);
      return true;
    }
    if (this.gone.has(b.id) || (b.splitGroupId !== undefined && this.gone.has(b.splitGroupId)))
      return false;
    const part: Part = { id: b.id, text: b.text, completed: b.completed === true };
    const group = b.splitGroupId && this.entries.find((e) => e.split && e.id === b.splitGroupId);
    if (group) {
      group.parts.push(part);
      group.quietSince = now;
      return true;
    }
    if (this.entry(b.splitGroupId ?? b.id)) return false;
    this.entries.unshift({
      id: b.splitGroupId ?? b.id,
      split: b.splitGroupId !== undefined,
      parts: [part],
      pinned: false,
      hover: new Set(),
      quietSince: now,
    });
    this.evict();
    return true;
  }

  /** Mark a blurb (or a split part) complete. False if it is not on the card. */
  complete(blurbId: string, now: number): boolean {
    const known = this.findPart(blurbId);
    if (!known) return false;
    this.markComplete(known.entry, known.part, now);
    return true;
  }

  /** Pin an entry on the card. False if it is unknown, in History, or no pin slot is free. */
  pin(entryId: string): boolean {
    const e = this.entry(entryId);
    if (!e) return false;
    if (e.pinned) return true;
    if (this.entries.filter((x) => x.pinned).length >= this.settings.maxBlurbs - 1) return false;
    e.pinned = true;
    return true;
  }

  /** Unpin an entry. Its collapse time starts again at `now`. */
  unpin(entryId: string, now: number): boolean {
    const e = this.entry(entryId);
    if (!e?.pinned) return false;
    e.pinned = false;
    e.quietSince = now;
    return true;
  }

  /** A pointer hover or keyboard focus starts or ends on an entry. */
  hover(entryId: string, via: HoverVia, on: boolean, now: number): void {
    const e = this.entry(entryId);
    if (!e) return;
    if (on) e.hover.add(via);
    else if (e.hover.delete(via) && e.hover.size === 0) e.quietSince = now;
  }

  /** Collapse every entry that is due at `now`. Returns the IDs that went to History. */
  tick(now: number): string[] {
    const due = this.entries.filter((e) => {
      const at = this.deadlineOf(e);
      return at !== undefined && at <= now;
    });
    for (const e of due) this.toHistory(e);
    return due.map((e) => e.id);
  }

  /** The next collapse time, or undefined when nothing can collapse. */
  nextDeadline(): number | undefined {
    const times = this.entries
      .map((e) => this.deadlineOf(e))
      .filter((t): t is number => t !== undefined);
    return times.length ? Math.min(...times) : undefined;
  }

  view(): StackView {
    return {
      blurbs: this.entries.map((e) => ({
        id: e.id,
        text: e.parts[0]?.text ?? '',
        pinned: e.pinned,
        completed: isComplete(e),
        ...(e.split ? { parts: e.parts.map((p) => ({ id: p.id, text: p.text })) } : {}),
      })),
      historyCount: this.historyTotal,
    };
  }

  /** IDs of the newest History entries, newest first. */
  history(): readonly string[] {
    return this.historyIds;
  }

  private entry(id: string): Entry | undefined {
    return this.entries.find((e) => e.id === id);
  }

  private findPart(id: string): { entry: Entry; part: Part } | undefined {
    for (const entry of this.entries) {
      const part = entry.parts.find((p) => p.id === id);
      if (part) return { entry, part };
    }
    return undefined;
  }

  private markComplete(e: Entry, p: Part, now: number): void {
    if (p.completed) return;
    p.completed = true;
    if (isComplete(e)) e.quietSince = now;
  }

  private deadlineOf(e: Entry): number | undefined {
    if (e.pinned || e.hover.size > 0 || !isComplete(e)) return undefined;
    return e.quietSince + this.settings.autoHideSec * 1000;
  }

  /** Move the oldest unpinned entries to History until `maxBlurbs` entries are left. */
  private evict(): void {
    while (this.entries.length > this.settings.maxBlurbs) {
      const oldest = [...this.entries].reverse().find((e) => !e.pinned);
      if (!oldest) return; // unreachable: pins are capped at maxBlurbs - 1
      this.toHistory(oldest);
    }
  }

  private toHistory(e: Entry): void {
    this.entries = this.entries.filter((x) => x !== e);
    this.historyIds = [e.id, ...this.historyIds].slice(0, HISTORY_IDS_MAX);
    this.gone.add(e.id);
    for (const p of e.parts) this.gone.add(p.id);
    this.historyTotal += 1;
  }
}

function isComplete(e: Entry): boolean {
  return e.parts.every((p) => p.completed);
}

/** What the card view asked for, decoded from its `view_emit`. */
export type CardInput =
  | { t: 'hover'; entryId: string; via: HoverVia; on: boolean }
  | { t: 'toggle_pin'; entryId: string }
  | { t: 'open_history' };

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

/**
 * Decode card input. The view sends `hover {blurbId, on, via}`, `click {target: 'pin', blurbId}`
 * and `click {target: 'history'}`. Anything else is dropped.
 */
export function decodeCardInput(kind: ViewEmitKind, payload: unknown): CardInput | undefined {
  if (!isObj(payload)) return undefined;
  if (kind === 'hover') {
    const { blurbId, on, via } = payload;
    if (typeof blurbId !== 'string' || typeof on !== 'boolean') return undefined;
    if (via !== 'pointer' && via !== 'focus') return undefined;
    return { t: 'hover', entryId: blurbId, via, on };
  }
  if (kind === 'click') {
    if (payload['target'] === 'history') return { t: 'open_history' };
    if (payload['target'] === 'pin' && typeof payload['blurbId'] === 'string')
      return { t: 'toggle_pin', entryId: payload['blurbId'] };
  }
  return undefined;
}
