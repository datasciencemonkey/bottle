import { describe, expect, it } from 'vitest';
import type { RecordingState } from '../shared/ipc';
import { createRecordToggle } from './record-toggle';

const state = (s: RecordingState['state']): RecordingState => ({ state: s, systemAudio: 'off' });

function rig(now: RecordingState['state'] | Error) {
  const calls: string[] = [];
  const refused: unknown[] = [];
  const toggle = createRecordToggle({
    call: async (command) => {
      calls.push(command);
      if (command === 'recording_state') {
        if (now instanceof Error) throw now;
        return state(now);
      }
      if (command === 'recording_start') throw { error: 'recording_off' };
      return state('stopping');
    },
    onRefused: (e) => void refused.push(e),
  });
  return { calls, refused, toggle };
}

describe('the ⌥R record toggle (ADR 0103)', () => {
  it('[ADR-0103] with no recording, ⌥R starts one with the preset (no instructions)', async () => {
    const r = rig('idle');
    await r.toggle();
    expect(r.calls).toEqual(['recording_state', 'recording_start']);
    // A refused start (here the knob went off) is reported, never thrown.
    expect(r.refused).toEqual([{ error: 'recording_off' }]);
  });

  it('[ADR-0103] while a recording starts or runs, ⌥R stops it', async () => {
    for (const s of ['recording', 'starting'] as const) {
      const r = rig(s);
      await r.toggle();
      expect(r.calls, s).toEqual(['recording_state', 'recording_stop']);
      expect(r.refused, s).toEqual([]);
    }
  });

  it('[ADR-0103] a recording that stops already, or a failed state read, gets no call', async () => {
    const r = rig('stopping');
    await r.toggle();
    expect(r.calls).toEqual(['recording_state']);
    const f = rig(new Error('host'));
    await f.toggle();
    expect(f.calls).toEqual(['recording_state']);
    expect(f.refused).toHaveLength(1);
  });
});
