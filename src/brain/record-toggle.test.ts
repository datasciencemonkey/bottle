import { describe, expect, it } from 'vitest';
import type { RecordingState } from '../shared/ipc';
import { createRecordToggle } from './record-toggle';

const state = (s: RecordingState['state']): RecordingState => ({ state: s, systemAudio: 'off' });

function rig(now: RecordingState['state'] | Error, home: string | undefined = 'work') {
  const calls: string[] = [];
  const args: unknown[] = [];
  const refused: unknown[] = [];
  const toggle = createRecordToggle({
    home: async () => home,
    call: async (command, a) => {
      calls.push(command);
      args.push(a);
      if (command === 'recording_state') {
        if (now instanceof Error) throw now;
        return state(now);
      }
      if (command === 'recording_start') throw { error: 'recording_off' };
      return state('stopping');
    },
    onRefused: (e) => void refused.push(e),
  });
  return { calls, args, refused, toggle };
}

describe('the ⌥R record toggle (ADR 0103)', () => {
  it('[ADR-0103] with no recording, ⌥R starts one with the preset (no instructions)', async () => {
    const r = rig('idle');
    await r.toggle();
    expect(r.calls).toEqual(['recording_state', 'recording_start']);
    // The brain's home profile goes with the start; Bottle never picks one.
    expect(r.args[1]).toEqual({ profile: 'work' });
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

describe('the ⌥R record toggle with no home profile (ADR 0103)', () => {
  it('[ADR-0103] with no home profile, ⌥R starts nothing and shows a plain refusal', async () => {
    for (const home of [undefined, '']) {
      const r = rig('idle', home);
      await r.toggle();
      expect(r.calls).toEqual(['recording_state']);
      expect(r.refused).toEqual([{ error: 'no_home_profile' }]);
    }
  });

  it('[ADR-0103] a stop needs no home profile', async () => {
    const r = rig('recording', undefined);
    await r.toggle();
    expect(r.calls).toEqual(['recording_state', 'recording_stop']);
    expect(r.refused).toEqual([]);
  });
});
