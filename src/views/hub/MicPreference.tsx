/**
 * The hub setting "Prefer the built-in mic when a Bluetooth headset connects" (CAP-22, #22).
 * With it on, the core builds the mic stream on the built-in mic when the default input is a
 * Bluetooth headset, so the headset stays in A2DP. The setting asks the OS for nothing
 * (CAP-17a). Default off.
 *
 * The value is the app setting `app.capture.preferBuiltInMic` (Appendix F, #453). The hub reads
 * and writes it with `app_setting_read` and `app_setting_write`; the host passes it to the mic.
 */
import { useEffect, useState } from 'react';
import { call } from '../../shared/ipc';

export const MIC_PREFERENCE_LABEL = 'Prefer the built-in mic when a Bluetooth headset connects';

/** Where the hub reads and writes the setting. */
export interface MicSettings {
  read(): Promise<boolean>;
  write(preferBuiltIn: boolean): Promise<void>;
}

const KEY = 'app.capture.preferBuiltInMic';

/** The app setting over IPC (#453). A refused or malformed read shows the setting off. */
export function ipcMicSettings(): MicSettings {
  return {
    read: async () => {
      try {
        const s: unknown = await call('app_setting_read', { key: KEY });
        return typeof s === 'object' && s !== null && (s as { value?: unknown }).value === true;
      } catch {
        return false;
      }
    },
    write: async (v) => {
      await call('app_setting_write', { key: KEY, value: v });
    },
  };
}

/** An in-memory store, default off (tests and previews). */
export function memoryMicSettings(initial = false): MicSettings {
  let value = initial;
  return {
    read: async () => value,
    write: async (v) => {
      value = v;
    },
  };
}

const appMicSettings = ipcMicSettings();

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
    try {
      await settings.write(next);
    } catch {
      // A refused write changes nothing: the box shows the stored value again.
    }
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
