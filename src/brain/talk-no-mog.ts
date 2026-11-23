/**
 * The talk key with no running MOG (#562, MB-06a, ADR 0095).
 *
 * With no open MOG window, the host sends the whole press as `hotkey:ptt` with an empty `mogId`
 * (`hotkey_host::PttForwarder`). On the key-down the brain starts the last-used MOG through T1,
 * as the hub Start does (`window_mog_open`). Then it sends each phase of the press again on the
 * bus with the ID of that MOG. So the talk-key target, the T12 consent gate and the capture
 * lease rules apply as for any other press. With no MOG at all, the hub shows
 * "Connect a workspace to start" and no capture starts. A hub that is closed (Bottle keeps running)
 * opens first on the MOGs area (#568), so the press always has a visible result. This needs no
 * contract change: the brain already shows the hub with the Tauri window API (`hub-window.ts`).
 *
 * Only a key-down starts a MOG: the key press is the user action. An `up`, `cancel` or
 * `double_tap` goes to the MOG that the key-down of its press started, or nowhere.
 * The last-used MOG is the last talk-key target of this run. With none (a new run), or when that
 * MOG was deleted, it is the first MOG of the hub list.
 */
import type { MogId } from '../shared/interfaces';
import type { HubNoticeLine } from '../shared/hub-mogs';
import { EVENTS } from '../shared/ipc';
import type { BrainBus } from './bus';
import { MogManagerError, type MogManager } from './components/mog-manager';

/** The hub toast for a talk-key press when no MOG exists. */
export const NO_MOG_TEXT = 'Connect a workspace to start';

const PTT = EVENTS['hotkey.ptt'].name;
const PHASES: ReadonlySet<string> = new Set(['down', 'up', 'cancel', 'double_tap']);

export interface TalkNoMogDeps {
  manager: Pick<MogManager, 'list' | 'start' | 'runState'>;
  /**
   * Open the `mog-<id>` window (`window_mog_open`, idempotent). For a MOG that T1 thinks is
   * running: the empty ID proves that no MOG window is open (TODO(#419)).
   */
  openWindow: (mogId: MogId) => Promise<unknown>;
  /** The last-used MOG of this run, open or closed (`TalkTarget.lastUsed`). */
  lastUsed: () => MogId | undefined;
  /** Show and focus the hub window (`showHub`). Called before a notice, so the user sees it. */
  showHub: () => unknown;
  /** Send the hub list again, with `notice` when there is one (`sendHubSnapshot`). */
  notify: (notice?: HubNoticeLine) => unknown;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

/** Wire the no-MOG press to `bus`. Returns `off`. */
export function wireTalkNoMog(bus: BrainBus, deps: TalkNoMogDeps): () => void {
  const notify = (notice?: HubNoticeLine) => {
    try {
      // #568: a notice is only useful if the hub is on screen. The hub switches to its MOGs area
      // when a list with a notice arrives.
      const shown =
        notice === undefined
          ? Promise.resolve()
          : Promise.resolve(deps.showHub()).catch(() => undefined);
      void shown.then(() => deps.notify(notice)).catch(() => undefined);
    } catch {
      // No hub: nothing to show.
    }
  };

  /** Start the last-used MOG. Undefined when none starts; the hub then shows why. */
  const startLastUsed = async (): Promise<MogId | undefined> => {
    let mogs;
    try {
      mogs = (await deps.manager.list()).mogs;
    } catch (e) {
      notify({
        kind: 'error',
        text: e instanceof MogManagerError ? e.message : 'Bottle cannot read the MOG list.',
      });
      return undefined;
    }
    const first = mogs[0];
    if (first === undefined) {
      notify({ kind: 'info', text: NO_MOG_TEXT });
      return undefined;
    }
    const last = deps.lastUsed();
    const pick = mogs.find((m) => m.mogId === last) ?? first;
    try {
      // TODO(#419): T1 keeps its run state in brain memory only. A MOG window that closed
      // outside T1 still reads `running`, and `start` does nothing for it, so open it here.
      const stale = deps.manager.runState(pick.mogId) === 'running';
      await deps.manager.start(pick.mogId);
      if (stale) await deps.openWindow(pick.mogId);
    } catch (e) {
      notify({
        kind: 'error',
        text: e instanceof MogManagerError ? e.message : `Bottle could not start ${pick.name}.`,
      });
      return undefined;
    }
    // The hub shows the MOG as running.
    notify();
    return pick.mogId;
  };

  /** The MOG of the press now: the result of its key-down. */
  let press: Promise<MogId | undefined> = Promise.resolve(undefined);

  return bus.on(PTT, (p) => {
    if (!isObj(p) || p['mogId'] !== '' || typeof p['phase'] !== 'string') return;
    const phase = p['phase'];
    if (!PHASES.has(phase)) return;
    // RTR-17a (ADR 0098): an ⌥G key-up keeps its `ask` flag.
    const ask = phase === 'up' && p['ask'] === true;
    if (phase === 'down') press = press.then(startLastUsed, startLastUsed);
    void press.then((mogId) => {
      if (mogId === undefined) return;
      bus.dispatch({
        event: PTT,
        payload: { mogId, phase, ...(ask ? { ask: true } : {}) },
      });
    });
  });
}
