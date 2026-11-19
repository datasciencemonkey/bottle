/**
 * The ⌥R record toggle (ADR 0103). The host sends `hotkey:record_toggle` only while the knob is
 * on, after the `cancel` that ended the hold. The brain reads the one recording of the device:
 * none runs, so it starts one with the Settings preset and the brain's home profile
 * (`recording_start {profile}`; with no home profile it starts nothing and reports
 * `no_home_profile`, because Bottle never picks a profile);
 * one starts or runs, so it stops it. Rust checks the knob and the consent again, so a refusal
 * (for example `recording_off` or `consent_required`) goes to `onRefused` and is never thrown.
 */
import type { ProfileName } from '../shared/interfaces';
import type { CommandArgs, CommandReturns } from '../shared/ipc';

type RecordingCommand = 'recording_state' | 'recording_start' | 'recording_stop';

export interface RecordToggleDeps {
  /** The brain's home profile now, or undefined (or '') when there is none. */
  home(): Promise<ProfileName | undefined>;
  call<C extends RecordingCommand>(command: C, args: CommandArgs[C]): Promise<CommandReturns[C]>;
  /** A failed state read, start or stop: the rejection (an `IpcError`). */
  onRefused?(error: unknown): void;
}

export function createRecordToggle(deps: RecordToggleDeps): () => Promise<void> {
  return async () => {
    try {
      const now = await deps.call('recording_state', {});
      if (now.state === 'idle') {
        const profile = await deps.home();
        if (profile === undefined || profile === '') {
          deps.onRefused?.({ error: 'no_home_profile' });
          return;
        }
        await deps.call('recording_start', { profile });
      } else if (now.state === 'recording' || now.state === 'starting')
        await deps.call('recording_stop', {});
    } catch (error) {
      deps.onRefused?.(error);
    }
  };
}
