import { describe, expect, it } from 'vitest';
import type { RecordingState } from '../shared/ipc';
import { createRecordToggle } from './record-toggle';
import { recordingProfile } from './recording-profile';

const state = (s: RecordingState['state']): RecordingState => ({ state: s, systemAudio: 'off' });

function rig(now: RecordingState['state'] | Error, ...homeArg: [string | undefined] | []) {
  const home = homeArg.length === 0 ? 'work' : homeArg[0];
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

  it('[ADR-0103] while a recording runs, ⌥R stops it', async () => {
    const r = rig('recording');
    await r.toggle();
    expect(r.calls).toEqual(['recording_state', 'recording_stop']);
    expect(r.refused).toEqual([]);
  });

  it('[ADR-0103] while a recording is still starting, ⌥R does nothing (as the hub stop() does)', async () => {
    const r = rig('starting');
    await r.toggle();
    expect(r.calls).toEqual(['recording_state']);
    expect(r.refused).toEqual([]);
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

/**
 * #594 review: ⌥R must read its profile through the same `recordingProfile` the hub Record
 * control uses (`recording-profile.ts`), not just the pressed MOG's own home. These tests wire
 * `createRecordToggle`'s `home()` to a real `recordingProfile` call, the same composition
 * `onRecordToggle` uses in `components/index.ts`.
 */
function rigWithHomes(home: string | undefined) {
  const calls: string[] = [];
  const args: unknown[] = [];
  const refused: unknown[] = [];
  const toggle = createRecordToggle({
    home: () => recordingProfile({ home: async () => home }),
    call: async (command, a) => {
      calls.push(command);
      args.push(a);
      if (command === 'recording_state') return state('idle');
      return state('starting');
    },
    onRefused: (e) => void refused.push(e),
  });
  return { calls, args, refused, toggle };
}

describe("⌥R shares the hub Record control's recordingProfile (ADR 0103, REC-H15, #594 review)", () => {
  it('[AC-1.8] with a home workspace, ⌥R starts with the home profile, the same as the hub', async () => {
    const r = rigWithHomes('ws1');
    await r.toggle();
    expect(r.calls).toEqual(['recording_state', 'recording_start']);
    expect(r.args[1]).toEqual({ profile: 'ws1' });
    expect(r.refused).toEqual([]);
  });

  it('no home workspace: ⌥R refuses with no_profile, not the generic no_home_profile', async () => {
    const r = rigWithHomes(undefined);
    await r.toggle();
    expect(r.calls).toEqual(['recording_state']);
    expect(r.refused).toEqual([{ error: 'no_profile' }]);
  });
});
