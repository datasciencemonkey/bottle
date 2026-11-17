/**
 * The hub setting "Prefer the built-in mic when a Bluetooth headset connects" (CAP-22, #22).
 * With it on, the core builds the mic stream on the built-in mic when the default input is a
 * Bluetooth headset, so the headset stays in A2DP. The setting asks the OS for nothing
 * (CAP-17a). Default off.
 *
 * The value is the app setting `app.capture.preferBuiltInMic` (ADR 0063). A view calls only
 * `view_emit` (ARC-2, MOG-08a), so the brain reads and writes it with `app_setting_*`.
 * TODO(#77): the Settings relay through the brain. Until then the hub keeps the value in memory.
 */
import { useEffect, useState } from 'react';

export const MIC_PREFERENCE_LABEL = 'Prefer the built-in mic when a Bluetooth headset connects';

/** Where the hub reads and writes the setting. */
export interface MicSettings {
  read(): Promise<boolean>;
  write(preferBuiltIn: boolean): Promise<void>;
}

/** An in-memory store, default off. TODO(#77): the brain relay of the app setting. */
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
