// @vitest-environment jsdom
/**
 * Call recording in the hub (ADR 0103, refs #65): the Settings section, the Record control and
 * the REC indicator. Thin view (ARC-2): it renders the brain model and sends `view_emit` only.
 * Fake values only.
 */
import { act } from 'react';
import {
  DEFAULT_NOTES_ENDPOINT,
  DEFAULT_TRANSCRIBE_ENDPOINT,
  DEFAULT_INSTRUCTIONS,
  HUB_RECORDING_EVENT,
  RECORDING_ERRORS,
  RECORDING_TEXT,
  RECORDING_VIEW_ID,
  type HubRecording,
} from '../../shared/recording-view';
import { inputEmits, render, tauri } from '../test-tauri';
import { RecordControl } from './RecordControl';
import { useHubRecording } from './recording-ipc';
import { RecordingSettings as Section } from './RecordingSettings';

/** The section as the hub mounts it: one model hook, then the section. */
function RecordingSettings() {
  return <Section model={useHubRecording()} />;
}

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const MY_FILES = '/Volumes/databricks/home/1234/someone@example.com/my_files';

function model(
  over: Partial<HubRecording> = {},
  s: Partial<NonNullable<HubRecording['settings']>> = {},
): HubRecording {
  return {
    platform: 'mac',
    settings: {
      enabled: false,
      consentAck: false,
      lengthMin: 60,
      transcribeEndpoint: DEFAULT_TRANSCRIBE_ENDPOINT,
      notesEndpoint: DEFAULT_NOTES_ENDPOINT,
      instructions: DEFAULT_INSTRUCTIONS,
      ...s,
    },
    storage: { status: 'my_files', path: MY_FILES },
    control: { phase: 'idle' },
    ...over,
  };
}

const emits = () =>
  inputEmits()
    .filter((e) => (e as { mogId: string }).mogId === RECORDING_VIEW_ID)
    .map((e) => (e as { payload: unknown }).payload);

async function click(el: Element | null | undefined) {
  await act(async () => (el as HTMLElement).click());
}

/** Set a field value the way React sees a user edit. */
async function type(el: Element | null, value: string) {
  const input = el as HTMLInputElement | HTMLTextAreaElement;
  const proto = Object.getPrototypeOf(input) as object;
  await act(async () => {
    Object.getOwnPropertyDescriptor(proto, 'value')?.set?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

async function show(m: HubRecording) {
  await tauri.emitTo('hub', HUB_RECORDING_EVENT, m);
}

describe('Call recording settings (hub)', () => {
  beforeEach(() => tauri.reset());

  it('[REC-V1] asks the brain for the model once it listens, and shows a quiet loading state', async () => {
    const { el } = await render(<RecordingSettings />);
    expect(emits()).toContainEqual({ action: 'recording.ready' });
    expect(el.querySelector('[data-view="recording-settings"] h2')?.textContent).toBe(
      'Call recording',
    );
    expect(el.querySelector('[aria-busy="true"]')).not.toBeNull();
  });

  it('[REC-V2] the knob is off by default and shows the key binding next to it', async () => {
    const { el } = await render(<RecordingSettings />);
    await show(model());
    // ui-1 (review of #602 round 3, P1): the knob is the shadcn/ui Switch (a `<button
    // role="switch" aria-checked>`), not a native `<input type="checkbox">`, so this reads
    // `aria-checked`, the same way WipeSettings.test.tsx reads its Radix controls.
    const knob = el.querySelector('[data-recording-knob]');
    expect(knob?.getAttribute('aria-checked')).toBe('false');
    expect(knob?.getAttribute('role')).toBe('switch');
    expect(el.textContent).toContain(RECORDING_TEXT.binding.mac);
    await show(model({ platform: 'windows' }));
    expect(el.textContent).toContain(RECORDING_TEXT.binding.windows);
  });

  it('[REC-V3] the first turn-on shows the consent notice once; one acknowledgement turns it on', async () => {
    const { el } = await render(<RecordingSettings />);
    await show(model());
    await click(el.querySelector('[data-recording-knob]'));
    expect(emits()).not.toContainEqual({ action: 'recording.enable', on: true });
    const notice = el.querySelector('[data-recording-consent]');
    expect(notice?.textContent).toContain(RECORDING_TEXT.consent);
    expect(notice?.textContent).toContain(RECORDING_TEXT.binding.mac);
    expect([...(notice?.querySelectorAll('button') ?? [])].map((b) => b.textContent)).toEqual([
      'Got it',
      'Not now',
    ]);
    // A keyboard or screen-reader user lands on the notice.
    expect(notice?.contains(document.activeElement)).toBe(true);
    await click(notice?.querySelector('button'));
    // One atomic ack: the brain saves the consent and turns the knob on in one write.
    expect(emits().at(-1)).toEqual({ action: 'recording.consent_ack' });
    expect(emits()).not.toContainEqual({ action: 'recording.enable', on: true });
  });

  it('[REC-V3] Not now closes the notice, keeps the knob off, and returns focus to the switch', async () => {
    const { el } = await render(<RecordingSettings />);
    await show(model());
    await click(el.querySelector('[data-recording-knob]'));
    const notNow = [...el.querySelectorAll('[data-recording-consent] button')].find(
      (b) => b.textContent === 'Not now',
    );
    await click(notNow);
    expect(el.querySelector('[data-recording-consent]')).toBeNull();
    expect(emits()).not.toContainEqual({ action: 'recording.consent_ack' });
    expect(emits()).not.toContainEqual({ action: 'recording.enable', on: true });
    expect(document.activeElement).toBe(el.querySelector('[data-recording-knob]'));
  });

  it('[REC-V4] the length radiogroup is labelled by its visible label', async () => {
    const { el } = await render(<RecordingSettings />);
    await show(model());
    const group = el.querySelector('[role="radiogroup"]');
    const id = group?.getAttribute('aria-labelledby') ?? '';
    expect(el.querySelector(`#${id}`)?.textContent).toBe('Stops after');
  });

  it('[REC-V3] after the acknowledgement the knob turns on and off with no notice', async () => {
    const { el } = await render(<RecordingSettings />);
    await show(model({}, { consentAck: true }));
    await click(el.querySelector('[data-recording-knob]'));
    expect(el.querySelector('[data-recording-consent]')).toBeNull();
    expect(emits().at(-1)).toEqual({ action: 'recording.enable', on: true });
    await show(model({}, { consentAck: true, enabled: true }));
    await click(el.querySelector('[data-recording-knob]'));
    expect(emits().at(-1)).toEqual({ action: 'recording.enable', on: false });
  });

  it('[REC-V4] the length offers 15/30/45/60/90 or custom; 60 is picked by default', async () => {
    const { el } = await render(<RecordingSettings />);
    await show(model());
    const radios = [...el.querySelectorAll<HTMLInputElement>('input[name="recording-length"]')];
    expect(radios.map((r) => r.value)).toEqual(['15', '30', '45', '60', '90', 'custom']);
    expect(radios.find((r) => r.checked)?.value).toBe('60');
    await click(radios[4]);
    expect(emits().at(-1)).toEqual({ action: 'recording.length', minutes: 90 });
    await click(radios[5]);
    const custom = el.querySelector<HTMLInputElement>('[data-recording-custom-length]');
    expect(custom).not.toBeNull();
    await type(custom, '120');
    await act(async () => custom?.form?.requestSubmit());
    expect(emits().at(-1)).toEqual({ action: 'recording.length', minutes: 120 });
  });

  it('[REC-V4] a saved custom length shows as custom', async () => {
    const { el } = await render(<RecordingSettings />);
    await show(model({}, { lengthMin: 75 }));
    const checked = el.querySelector<HTMLInputElement>('input[name="recording-length"]:checked');
    expect(checked?.value).toBe('custom');
    expect(el.querySelector<HTMLInputElement>('[data-recording-custom-length]')?.value).toBe('75');
  });

  it('[REC-V5] storage shows My Files when the probe finds it', async () => {
    const { el } = await render(<RecordingSettings />);
    await show(model());
    const storage = el.querySelector('[data-recording-storage]');
    expect(storage?.textContent).toContain('My Files');
    expect(storage?.textContent).not.toContain(RECORDING_TEXT.myFilesMissing);
  });

  it('[REC-V5] without My Files: a volume path, or a catalog.schema with an explicit create confirm', async () => {
    const { el } = await render(<RecordingSettings />);
    await show(model({ storage: { status: 'unavailable' } }));
    const storage = el.querySelector('[data-recording-storage]');
    expect(storage?.textContent).toContain(RECORDING_TEXT.myFilesMissing);
    const input =
      storage?.querySelector<HTMLInputElement>('[data-recording-storage-input]') ?? null;
    await type(input, '/Volumes/main/team/audio');
    await act(async () => input?.form?.requestSubmit());
    expect(emits().at(-1)).toEqual({
      action: 'recording.storage_volume',
      path: '/Volumes/main/team/audio',
    });
    await type(input, 'main.team');
    await act(async () => input?.form?.requestSubmit());
    expect(emits().at(-1)).not.toMatchObject({ action: 'recording.volume_create' });
    const confirm = el.querySelector('[data-recording-volume-create]');
    expect(confirm?.textContent).toBe('Create volume bottle_audio');
    await click(confirm);
    expect(emits().at(-1)).toEqual({
      action: 'recording.volume_create',
      catalog: 'main',
      schema: 'team',
    });
  });

  it('[REC-V5] a storage error shows in plain words', async () => {
    const { el } = await render(<RecordingSettings />);
    await show(model({ storage: { status: 'unavailable', error: RECORDING_TEXT.badVolume } }));
    expect(el.querySelector('[data-recording-storage] [role="alert"]')?.textContent).toBe(
      RECORDING_TEXT.badVolume,
    );
  });

  it('[REC-V6] Advanced holds the transcribe and notes models, with their defaults; each saves on change', async () => {
    const { el } = await render(<RecordingSettings />);
    await show(model());
    const adv = el.querySelector('[data-recording-advanced]');
    // ui-1 (review of #602 round 3, P2): a Collapsible trigger (`data-slot`, same convention as
    // every shadcn/ui primitive here), not a native `<details>`/`<summary>` any more.
    expect(adv?.querySelector('[data-slot="collapsible-trigger"]')?.textContent).toBe('Advanced');
    const tr =
      adv?.querySelector<HTMLInputElement>('[data-recording-endpoint="transcribe"]') ?? null;
    const notes = adv?.querySelector<HTMLInputElement>('[data-recording-endpoint="notes"]') ?? null;
    expect(tr?.value).toBe(DEFAULT_TRANSCRIBE_ENDPOINT);
    expect(notes?.value).toBe(DEFAULT_NOTES_ENDPOINT);
    await type(tr, 'my-transcriber');
    await act(async () => tr?.dispatchEvent(new FocusEvent('focusout', { bubbles: true })));
    expect(emits().at(-1)).toEqual({
      action: 'recording.endpoint',
      which: 'transcribe',
      endpoint: 'my-transcriber',
    });
    await type(notes, 'my-endpoint');
    await act(async () => notes?.dispatchEvent(new FocusEvent('focusout', { bubbles: true })));
    expect(emits().at(-1)).toEqual({
      action: 'recording.endpoint',
      which: 'notes',
      endpoint: 'my-endpoint',
    });
  });

  it('[REC-V7] the instructions show the preset; Reset to default sends the default', async () => {
    const { el } = await render(<RecordingSettings />);
    await show(model({}, { instructions: 'Only decisions.' }));
    const area = el.querySelector<HTMLTextAreaElement>('[data-recording-instructions]');
    expect(area?.value).toBe('Only decisions.');
    await click(el.querySelector('[data-recording-instructions-reset]'));
    expect(emits().at(-1)).toEqual({
      action: 'recording.instructions',
      text: DEFAULT_INSTRUCTIONS,
    });
    await show(model());
    expect(
      el.querySelector<HTMLButtonElement>('[data-recording-instructions-reset]')?.disabled,
    ).toBe(true);
  });

  it('[REC-V8] Settings has no warehouse or compute setting', async () => {
    const { el } = await render(<RecordingSettings />);
    await show(model());
    expect(el.querySelector('select')).toBeNull();
    expect(el.textContent).not.toMatch(/warehouse|serverless|compute|runs on/i);
  });
});

describe('Record control (hub)', () => {
  beforeEach(() => tauri.reset());

  it('[REC-V9] Record is off with a plain hint while the knob is off', async () => {
    const { el } = await render(<RecordControl model={model()} />);
    expect(el.querySelector<HTMLButtonElement>('[data-recording-start]')?.disabled).toBe(true);
    expect(el.textContent).toContain(RECORDING_TEXT.turnOn);
  });

  it('[REC-V9] Record starts with the preset; edited instructions go with that recording only', async () => {
    const m = model({}, { enabled: true, consentAck: true });
    const { el } = await render(<RecordControl model={m} />);
    await click(el.querySelector('[data-recording-start]'));
    expect(emits().at(-1)).toEqual({ action: 'recording.start' });
    const toggle = el.querySelector('[data-recording-edit-toggle]');
    expect(toggle?.textContent).toBe('Edit instructions for this recording');
    expect(toggle?.getAttribute('aria-expanded')).toBe('false');
    await click(toggle);
    expect(toggle?.getAttribute('aria-expanded')).toBe('true');
    const area = el.querySelector<HTMLTextAreaElement>('[data-recording-edit] textarea');
    expect(area?.value).toBe(DEFAULT_INSTRUCTIONS);
    await type(area, 'Only action items.');
    await click(el.querySelector('[data-recording-start]'));
    expect(emits().at(-1)).toEqual({
      action: 'recording.start',
      instructions: 'Only action items.',
    });
  });

  it('[REC-V10] while recording: a REC indicator with the time left, and Stop', async () => {
    const m = model(
      { control: { phase: 'recording', startedAtMs: 0, endsAtMs: 3_600_000 } },
      { enabled: true, consentAck: true },
    );
    const { el } = await render(<RecordControl model={m} now={() => 1_000} />);
    const rec = el.querySelector('[data-recording-indicator]');
    expect(rec?.textContent).toContain('REC');
    expect(rec?.textContent).toContain('59:59 left');
    expect(el.querySelector('[data-recording-start]')).toBeNull();
    await click(el.querySelector('[data-recording-stop]'));
    expect(emits().at(-1)).toEqual({ action: 'recording.stop' });
  });

  it('[REC-V11] an error shows its plain text and its one action', async () => {
    const onSettings = vi.fn();
    const m = model(
      {
        control: { phase: 'recording', endsAtMs: 10 },
        error: RECORDING_ERRORS.system_audio_denied,
      },
      { enabled: true, consentAck: true },
    );
    const { el, root } = await render(<RecordControl model={m} onOpenSettings={onSettings} />);
    const alert = el.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain(RECORDING_ERRORS.system_audio_denied.text);
    await click(alert?.querySelector('button'));
    expect(emits().at(-1)).toEqual({ action: 'recording.system_audio' });
    await act(async () =>
      root.render(
        <RecordControl
          model={{ ...m, error: RECORDING_ERRORS.volume_permission }}
          onOpenSettings={onSettings}
        />,
      ),
    );
    await click(el.querySelector('[role="alert"] button'));
    expect(onSettings).toHaveBeenCalledTimes(1);
  });
});
