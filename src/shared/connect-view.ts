/**
 * The hub model of "Connect a workspace" (#162; W-07a, W-07f, W-07g, W-07h, W-07i, W-15). The
 * brain builds it (`src/brain/connect-wiring.ts`) and sends it to the hub only; the thin hub
 * view `ConnectWorkspace` shows it and sends clicks with `view_emit` (ARC-2, MOG-08a). It holds
 * a workspace host and a profile name, never a token. It is not a contract file. TODO(#525):
 * the event name moves into the contract with the other hub events.
 */

/** The brain-to-hub event of the model. TODO(#525). */
export const CONNECT_EVENT = 'hub:connect_workspace';
/** The `view_emit` id of the form clicks. */
export const CONNECT_VIEW_ID = 'connect';

/** The click targets of the form (the `target` of the `view_emit` payload). */
export type ConnectTarget =
  | 'connect_open'
  | 'connect_url'
  | 'connect_start'
  | 'connect_cancel'
  | 'connect_reopen'
  | 'connect_retry'
  /** Close the flow (the Cancel of a Settings-opened step, or Done). The brain answers `closed`. */
  | 'connect_close';

/** The one action of a failed step (W-07h, Table W-ERR). */
export interface ConnectAction {
  label: string;
  target: 'connect_retry' | 'connect_open';
}

type ConnectStep =
  /** W-07a: ask for the workspace URL. `firstRun` (W-07f): no usable profile exists. */
  | { step: 'url'; firstRun: boolean; error?: string }
  /**
   * W-07g, W-07i: the host, an editable proposed name, and the PAT/SP notice. `taken`: the
   * other profile names the brain already knows of, so the view can refuse a duplicate at once
   * (an inline message), without waiting for the `connect_start` round trip. Optional: a sender
   * that omits it is read as "none known yet".
   */
  | {
      step: 'name';
      host: string;
      proposedName: string;
      otherAuthNotice: boolean;
      taken?: readonly string[];
      error?: string;
    }
  /** W-07h: `databricks auth login` runs; finish in the browser. */
  | { step: 'waiting'; host: string; profile: string }
  | { step: 'connected'; host: string; profile: string }
  /** W-07h: the Table W-ERR text and one action. */
  | { step: 'failed'; host: string; profile: string; text: string; action: ConnectAction };

/**
 * `inline` (W-07e): the flow was opened from the Settings row, so Settings shows it under that
 * row and the setup region does not. Without it, the setup region shows it (first run, W-07f).
 */
export type ConnectModel =
  | (ConnectStep & { inline?: boolean })
  /**
   * The brain's explicit "nothing to open" state: a usable profile exists (W-07f), or the flow
   * ended (Done, Cancel, or the connected step timed out). No view renders anything for it;
   * `connect_open` (the Settings row, "Connect another workspace…") is the way to leave it.
   */
  | { step: 'closed' };

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const str = (v: unknown): v is string => typeof v === 'string';
const optStr = (v: unknown) => v === undefined || str(v);
const optStrArray = (v: unknown) => v === undefined || (Array.isArray(v) && v.every(str));

/**
 * The CLI flag-value rule `profile_login.rs::plain_value` mirrors, so the view can tell the user
 * a typed name is bad before the `connect_start` round trip: `[A-Za-z0-9._-]`, not starting with
 * `-`, 1 to 64 characters. The host (`profile_login`) checks the same rule again; this is only
 * the earlier, friendlier message.
 */
const PLAIN_VALUE = /^[A-Za-z0-9._][A-Za-z0-9._-]{0,63}$/;

/** True for a name the CLI would accept as a plain flag value (W-07g). */
export function isValidProfileName(name: string): boolean {
  return PLAIN_VALUE.test(name);
}

/** The model, or `undefined` for a payload that is not one. */
export function asConnectModel(v: unknown): ConnectModel | undefined {
  if (!isObj(v)) return undefined;
  if (v['inline'] !== undefined && typeof v['inline'] !== 'boolean') return undefined;
  switch (v['step']) {
    case 'url':
      return typeof v['firstRun'] === 'boolean' && optStr(v['error'])
        ? (v as ConnectModel)
        : undefined;
    case 'name':
      return str(v['host']) &&
        str(v['proposedName']) &&
        typeof v['otherAuthNotice'] === 'boolean' &&
        optStrArray(v['taken']) &&
        optStr(v['error'])
        ? (v as ConnectModel)
        : undefined;
    case 'waiting':
    case 'connected':
      return str(v['host']) && str(v['profile']) ? (v as ConnectModel) : undefined;
    case 'failed': {
      const a = v['action'];
      return str(v['host']) &&
        str(v['profile']) &&
        str(v['text']) &&
        isObj(a) &&
        str(a['label']) &&
        (a['target'] === 'connect_retry' || a['target'] === 'connect_open')
        ? (v as ConnectModel)
        : undefined;
    }
    case 'closed':
      return { step: 'closed' };
    default:
      return undefined;
  }
}
