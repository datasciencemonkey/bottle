/**
 * The hub setting "Prefer the built-in mic when a Bluetooth headset connects" (CAP-22, #22).
 * With it on, the core builds the mic stream on the built-in mic when the default input is a
 * Bluetooth headset, so the headset stays in A2DP. The setting asks the OS for nothing
 * (CAP-17a). Default off.
 *
 * TODO(#453): there is no app-wide setting IPC yet. Until it lands, the hub keeps the value in
 * memory and the core does not see it.
 */
import { useEffect, useState } from 'react';

export const MIC_PREFERENCE_LABEL = 'Prefer the built-in mic when a Bluetooth headset connects';

/** Where the hub reads and writes the setting. */
export interface MicSettings {
  read(): Promise<boolean>;
  write(preferBuiltIn: boolean): Promise<void>;
}

/** An in-memory store, default off. TODO(#453): replace with the app setting IPC. */
export function memoryMicSettings(initial = false): MicSettings {
  let value = initial;
  return {
    read: async () => value,
    write: async (v) => {
      value = v;
    },
  };
}

const appMicSettings = memoryMicSettings();

export function MicPreference({ settings = appMicSettings }: { settings?: MicSettings }) {
  const [on, setOn] = useState(false);
  useEffect(() => {
    let live = true;
    void settings.read().then((v) => live && setOn(v));
    return () => {
      live = false;
    };
  }, [settings]);

  const toggle = async (next: boolean) => {
    await settings.write(next);
    setOn(await settings.read());
  };

  return (
    <section data-view="mic-preference" aria-labelledby="mic-preference-title">
      <h2 id="mic-preference-title">Microphone</h2>
      <input
        id="prefer-built-in-mic"
        type="checkbox"
        name="preferBuiltInMic"
        checked={on}
        onChange={(e) => void toggle(e.currentTarget.checked)}
      />
      <label htmlFor="prefer-built-in-mic">{MIC_PREFERENCE_LABEL}</label>
    </section>
  );
}
