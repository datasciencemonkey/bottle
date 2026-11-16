/**
 * The brain side of the hub notice (#27, MB-08a, MB-08b, ADR 0044).
 *
 * The brain sends the notice state to the hub only (ARC-5). The hub sends `notice.ready` when it
 * opens and `notice.accept` on a Continue click. Only the hub window can accept; T12
 * ConsentService refuses a version that is not the current one.
 * TODO(#393): the event name and the actions move into the contract.
 */
import { emitTo } from '@tauri-apps/api/event';
import { asNoticeAction, HUB_NOTICE_EVENT, type HubNotice } from '../shared/hub-notice';
import { onViewInput, type BrainBus } from './bus';
import type { ConsentService } from './components/consent-service';

/** Send the notice state to the hub window only. */
export function sendHubNotice(state: HubNotice): Promise<void> {
  return emitTo('hub', HUB_NOTICE_EVENT, state);
}

/** Wire the hub notice actions to `consent`, and send the state to the hub once now. */
export function wireNotice(
  bus: BrainBus,
  consent: ConsentService,
  send: (state: HubNotice) => Promise<unknown> = sendHubNotice,
): () => void {
  const publish = async () => {
    try {
      await send(await consent.noticeState());
    } catch {
      // No hub is open. The hub asks again with notice.ready when it opens.
    }
  };
  void publish();
  return onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.kind !== 'click') return;
    const action = asNoticeAction(input.payload);
    if (!action) return;
    void (async () => {
      if (action.action === 'notice.accept') await consent.acceptNotice(action.noticeVersion);
      await publish();
    })();
  });
}
