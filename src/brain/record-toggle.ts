/**
 * The ⌥R record toggle (ADR 0103). The host sends `hotkey:record_toggle` only while the knob is
 * on, after the `cancel` that ended the hold. The brain reads the one recording of the device:
 * none runs, so it starts one with the Settings preset (`recording_start` with no instructions);
 * one starts or runs, so it stops it. Rust checks the knob and the consent again, so a refusal
 * (for example `recording_off` or `consent_required`) goes to `onRefused` and is never thrown.
 */
import type { CommandArgs, CommandReturns } from '../shared/ipc';

type RecordingCommand = 'recording_state' | 'recording_start' | 'recording_stop';

export interface RecordToggleDeps {
  call<C extends RecordingCommand>(command: C, args: CommandArgs[C]): Promise<CommandReturns[C]>;
  /** A failed state read, start or stop: the rejection (an `IpcError`). */
  onRefused?(error: unknown): void;
}

export function createRecordToggle(deps: RecordToggleDeps): () => Promise<void> {
  return async () => {
    try {
      const now = await deps.call('recording_state', {});
      if (now.state === 'idle') await deps.call('recording_start', {});
      else if (now.state === 'recording' || now.state === 'starting')
        await deps.call('recording_stop', {});
    } catch (error) {
      deps.onRefused?.(error);
    }
  };
}
