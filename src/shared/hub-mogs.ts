/**
 * The hub MOG list and the hub MOG actions (#26, MB-01, MB-03a, MB-03b, ADR in the PR).
 *
 * The brain (T1 MogManager) and the hub view both read this file. It holds names and shape
 * checks only, no state. It is not a contract file.
 * TODO(#407): move `HUB_MOGS_EVENT`, `HubMogList` and the MOG actions into `src/shared/ipc.ts`
 * (`EVENTS['hub.mogs']` and a hub-level view input).
 */

/** The brain-to-view event that carries the MOG list to the hub. TODO(#407): `EVENTS`. */
export const HUB_MOGS_EVENT = 'hub:mogs';

/**
 * The `mogId` of a hub-level `view_emit`, shared with the #27 notice. It cannot be a MOG ID,
 * because MOG IDs are UUIDs. TODO(#407): a hub-level view input with no `mogId`.
 */
export { HUB_SCOPE } from './hub-notice';

/** One row of the hub MOG list. */
export interface HubMogRow {
  mogId: string;
  name: string;
  /** The home profile name (never a token). */
  home: string;
  persona: string;
  running: boolean;
  /**
   * #ui-4: `mog.ui.variant`, so the row swatch and the lamp, the card and Conversations show the
   * same colour (`src/shared/mog-palette.ts`). Never a raw hex: the view resolves the id.
   */
  colorVariant: string;
}

/** A line for the hub status region. The brain writes the text; the view shows it as is. */
export interface HubNoticeLine {
  kind: 'error' | 'info';
  text: string;
}

/** What the hub shows: every defined MOG, and the running count against the cap. */
export interface HubMogList {
  mogs: HubMogRow[];
  /** Config IDs that the core cannot read. Bottle keeps these files (ADR 0035). */
  unreadable: string[];
  runningCount: number;
  maxRunning: number;
  notice?: HubNoticeLine;
  /**
   * #ui-4 (review #603, MEDIUM finding): the MOG right-click menu's "Change home workspace…"
   * asks the hub to open this MOG's own Edit form, not only show the hub
   * (`hub-mogs-wiring.ts`'s `openEdit`). One-shot: `token` changes on every ask, so the hub view
   * (`MogList.tsx`) can tell a fresh ask from the same `mogId` from the one it already opened.
   */
  openEdit?: { mogId: string; token: number };
}

/** The hub MOG actions, sent as `view_emit` kind `click` with mogId `HUB_SCOPE`. */
export type MogAction =
  | { action: 'mogs.ready' }
  | { action: 'mog.create'; name: string; home: string }
  | { action: 'mog.edit'; mogId: string; name: string; home: string; persona: string }
  | { action: 'mog.duplicate'; mogId: string }
  | { action: 'mog.start'; mogId: string }
  | { action: 'mog.stop'; mogId: string }
  | { action: 'mog.delete'; mogId: string; confirmed: true }
  /** #ui-4: the row swatch picker. `colorVariant` is a palette id (`mog-palette.ts`). */
  | { action: 'mog.set_color'; mogId: string; colorVariant: string };

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const isStr = (v: unknown): v is string => typeof v === 'string';
const isCount = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && v >= 0;

/** Shape check of a hub MOG action. Unknown fields are dropped. */
export function asMogAction(p: unknown): MogAction | undefined {
  if (!isObj(p)) return undefined;
  const { action, mogId, name, home, persona } = p;
  switch (action) {
    case 'mogs.ready':
      return { action };
    case 'mog.create':
      return isStr(name) && isStr(home) ? { action, name, home } : undefined;
    case 'mog.edit':
      return isStr(mogId) && isStr(name) && isStr(home) && isStr(persona)
        ? { action, mogId, name, home, persona }
        : undefined;
    case 'mog.duplicate':
    case 'mog.start':
    case 'mog.stop':
      return isStr(mogId) ? { action, mogId } : undefined;
    case 'mog.delete':
      return isStr(mogId) && p['confirmed'] === true
        ? { action, mogId, confirmed: true }
        : undefined;
    case 'mog.set_color': {
      const colorVariant = p['colorVariant'];
      return isStr(mogId) && isStr(colorVariant) ? { action, mogId, colorVariant } : undefined;
    }
    default:
      return undefined;
  }
}

function isRow(v: unknown): v is HubMogRow {
  return (
    isObj(v) &&
    isStr(v['mogId']) &&
    isStr(v['name']) &&
    isStr(v['home']) &&
    isStr(v['persona']) &&
    typeof v['running'] === 'boolean' &&
    isStr(v['colorVariant'])
  );
}

/** Shape check of a `hub:mogs` payload. */
export function asHubMogList(p: unknown): HubMogList | undefined {
  if (!isObj(p)) return undefined;
  const { mogs, unreadable, runningCount, maxRunning, notice, openEdit } = p;
  if (!Array.isArray(mogs) || !mogs.every(isRow)) return undefined;
  if (!Array.isArray(unreadable) || !unreadable.every(isStr)) return undefined;
  if (!isCount(runningCount) || !isCount(maxRunning)) return undefined;
  if (
    notice !== undefined &&
    !(
      isObj(notice) &&
      (notice['kind'] === 'error' || notice['kind'] === 'info') &&
      isStr(notice['text'])
    )
  )
    return undefined;
  if (
    openEdit !== undefined &&
    !(isObj(openEdit) && isStr(openEdit['mogId']) && typeof openEdit['token'] === 'number')
  )
    return undefined;
  return p as unknown as HubMogList;
}
