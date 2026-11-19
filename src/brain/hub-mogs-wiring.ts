/**
 * The brain side of the hub MOG list (#26, MB-01, MB-03a, MB-03b).
 *
 * The brain sends the list to the hub only (ARC-5). The hub sends `mogs.ready` when it opens and
 * one MOG action per user action. Only the hub window can run a MOG action. After each action the
 * brain sends the list again, with the result text of the action as `notice`.
 * TODO(#407): the event name and the actions move into the contract.
 */
import { emitTo } from '@tauri-apps/api/event';
import {
  asMogAction,
  HUB_MOGS_EVENT,
  HUB_SCOPE,
  type HubMogList,
  type HubNoticeLine,
  type MogAction,
} from '../shared/hub-mogs';
import { onViewInput, type BrainBus } from './bus';
import { MogManagerError, type MogManager } from './components/mog-manager';

/** Send the MOG list to the hub window only. */
export function sendHubMogs(list: HubMogList): Promise<void> {
  return emitTo('hub', HUB_MOGS_EVENT, list);
}

async function act(
  m: MogManager,
  a: MogAction,
  // #ui-4 (review #603, finding 3): called after a successful `mog.edit`/`mog.set_color`, so
  // the caller can push the fresh identity to an open MOG and its card (`talkTarget.reidentify`
  // in `components/index.ts`), not only update the hub row.
  onIdentityChanged?: (mogId: string) => void,
): Promise<HubNoticeLine | undefined> {
  switch (a.action) {
    case 'mogs.ready':
      return undefined;
    case 'mog.create':
      await m.create({ name: a.name, home: a.home });
      return undefined;
    case 'mog.edit':
      await m.edit(a.mogId, { name: a.name, home: a.home, persona: a.persona });
      onIdentityChanged?.(a.mogId);
      return undefined;
    case 'mog.set_color':
      await m.edit(a.mogId, { colorVariant: a.colorVariant });
      onIdentityChanged?.(a.mogId);
      return undefined;
    case 'mog.duplicate':
      await m.duplicate(a.mogId);
      return undefined;
    case 'mog.start':
      await m.start(a.mogId);
      return undefined;
    case 'mog.stop':
      await m.stop(a.mogId);
      return undefined;
    case 'mog.delete': {
      const name = (await m.list()).mogs.find((s) => s.mogId === a.mogId)?.name ?? 'the MOG';
      await m.remove(a.mogId, { confirmed: true });
      return {
        kind: 'info',
        text: `Deleted ${name}. Its memory and history stay until you use Wipe in Settings.`,
      };
    }
  }
}

async function snapshot(m: MogManager, notice?: HubNoticeLine): Promise<HubMogList> {
  const base = { runningCount: m.runningCount(), maxRunning: m.maxRunning };
  try {
    const l = await m.list();
    const mogs = l.mogs.map((s) => ({
      mogId: s.mogId,
      name: s.name,
      home: s.binding.home,
      persona: s.persona,
      running: s.runState === 'running',
      colorVariant: s.colorVariant,
    }));
    return { mogs, unreadable: l.unreadable, ...base, ...(notice ? { notice } : {}) };
  } catch (e) {
    const text = e instanceof MogManagerError ? e.message : 'Bottle cannot read the MOG list.';
    return { mogs: [], unreadable: [], ...base, notice: notice ?? { kind: 'error', text } };
  }
}

/**
 * Send the list to the hub outside a hub action, with an optional `notice` (#562: the talk key
 * started a MOG, or there is no MOG). False when no hub is open.
 */
export async function sendHubSnapshot(
  manager: MogManager,
  notice?: HubNoticeLine,
  send: (list: HubMogList) => Promise<unknown> = sendHubMogs,
): Promise<boolean> {
  try {
    await send(await snapshot(manager, notice));
    return true;
  } catch {
    return false;
  }
}

/**
 * The function `wireHubMogs` returns: calling it stops the wiring, same as before. `openEdit`
 * (#ui-4, review #603, MEDIUM finding) asks the hub to open one MOG's own Edit form on the next
 * list push, for "Change home workspace…" on the MOG right-click menu.
 */
export interface HubMogsDisposer {
  (): void;
  openEdit(mogId: string): void;
}

/** Wire the hub MOG actions to `manager`, and send the list to the hub once now. */
export function wireHubMogs(
  bus: BrainBus,
  manager: MogManager,
  send: (list: HubMogList) => Promise<unknown> = sendHubMogs,
  onIdentityChanged?: (mogId: string) => void,
): HubMogsDisposer {
  let live = true;
  // #ui-4 (review #603, MEDIUM finding): held the same way `hub-conversations-wiring.ts` holds
  // a pending Conversations open, so a hub that is not open yet (or whose send races the open)
  // still gets it, once, on the next push that reaches it.
  let pendingEdit: { mogId: string; token: number } | undefined;
  let editToken = 0;
  const publish = async (notice?: HubNoticeLine) => {
    const list = await snapshot(manager, notice);
    if (!live) return;
    const edit = pendingEdit;
    const withEdit = edit ? { ...list, openEdit: edit } : list;
    try {
      await send(withEdit);
      // Consumed only once actually sent; a send failure (no hub open) keeps it for the next try.
      if (edit) pendingEdit = undefined;
    } catch {
      // No hub is open. The hub asks again with mogs.ready when it opens.
    }
  };
  void publish();
  const off = onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.mogId !== HUB_SCOPE || input.kind !== 'click') return;
    const action = asMogAction(input.payload);
    if (!action) return;
    void (async () => {
      let notice: HubNoticeLine | undefined;
      try {
        notice = await act(manager, action, onIdentityChanged);
      } catch (e) {
        // A MogManagerError holds fixed text only; any other error gets a fixed text.
        notice = {
          kind: 'error',
          text: e instanceof MogManagerError ? e.message : 'Bottle could not do that. Try again.',
        };
      }
      await publish(notice);
    })();
  });
  const dispose = (() => {
    live = false;
    off();
  }) as HubMogsDisposer;
  dispose.openEdit = (mogId) => {
    pendingEdit = { mogId, token: ++editToken };
    void publish();
  };
  return dispose;
}
