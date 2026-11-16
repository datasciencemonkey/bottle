/**
 * The first-run notice gate on `capture_start` (#27, MB-08a, MB-08b, ADR 0044).
 *
 * Only the brain window may call `capture_start` (4.5), and only this file calls it: a test fails
 * if another brain file does. Before the user accepts the current notice version, the gate sends
 * no command, shows the notice in the hub and rejects with the contract error
 * `{ error: 'consent_required' }`.
 * TODO(#14, #4): the capture lifecycle starts every capture through `startCapture`.
 * TODO(#393): also open and focus the hub window when the gate blocks (no hub window command yet).
 */
import type { HubNotice } from '../shared/hub-notice';
import { call, type CommandArgs, type CommandReturns, type IpcError } from '../shared/ipc';
import type { ConsentService } from './components/consent-service';
import { sendHubNotice } from './notice-wiring';

export interface CaptureGateDeps {
  consent: Pick<ConsentService, 'noticeState'>;
  /** Called with the notice state when the gate blocks. Default: send it to the hub. */
  onBlocked?: (state: HubNotice) => unknown;
  /** The `capture_start` call. Default: the IPC `call`. */
  invoke?: (
    name: 'capture_start',
    args: CommandArgs['capture_start'],
  ) => Promise<CommandReturns['capture_start']>;
}

export const CONSENT_REQUIRED: IpcError = { error: 'consent_required' };

/** Start capture only after the user accepted the current notice version. */
export async function startCapture(
  deps: CaptureGateDeps,
  args: CommandArgs['capture_start'],
): Promise<CommandReturns['capture_start']> {
  const state = await deps.consent.noticeState();
  if (!state.accepted) {
    try {
      await (deps.onBlocked ?? sendHubNotice)(state);
    } catch {
      // No hub is open: capture stays blocked all the same.
    }
    throw { ...CONSENT_REQUIRED };
  }
  return (deps.invoke ?? call)('capture_start', args);
}
