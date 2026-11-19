/**
 * Settings → Call recording in the hub (thin; ADR 0103, refs #65, ARC-2).
 *
 * It renders the brain model (`hub:recording`) and sends each change with `view_emit` (id
 * `recording`). The brain checks every value and owns every text; the core checks the knob again
 * before any capture starts. The first turn-on shows the consent notice once, with one
 * acknowledgement. The view never builds a path or a URL; it only sends what the user typed.
 *
 * ui-1 (review of #602 round 3, P1): the knob, the length and the storage rows now sit in a
 * `SettingsGroup`/`SettingsRow`, the same grouped, rounded-box layout `MicPreference` already
 * used, so Settings shows one layout for a toggle-plus-control section, not a bespoke one just
 * for this section. The knob is the shadcn/ui `Switch` (`MicPreference`'s control), not the
 * red `input.hub-switch` this used to render: Settings had two different switch styles side by
 * side before this.
 */
import { ChevronDownIcon } from 'lucide-react';
import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import {
  CATALOG_SCHEMA,
  DEFAULT_INSTRUCTIONS,
  LENGTH_CHOICES,
  LENGTH_MAX,
  LENGTH_MIN,
  RECORDING_TEXT,
  type HubRecording,
  type RecordingSettingsView,
  type StorageView,
} from '../../shared/recording-view';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../ui/collapsible';
import { Switch } from '../ui/switch';
import { SettingsGroup, SettingsRow } from './SettingsGroup';
import { sendRecording } from './recording-ipc';

const isPreset = (m: number) => (LENGTH_CHOICES as readonly number[]).includes(m);

function Knob({ model, settings }: { model: HubRecording; settings: RecordingSettingsView }) {
  const [asking, setAsking] = useState(false);
  const id = useId();
  const knob = useRef<HTMLButtonElement>(null);
  const ok = useRef<HTMLButtonElement>(null);
  const binding = RECORDING_TEXT.binding[model.platform];
  const open = asking && !settings.consentAck;
  // The notice takes focus when it opens, so a keyboard or screen-reader user lands on it.
  useEffect(() => {
    if (open) ok.current?.focus();
  }, [open]);
  const close = () => {
    setAsking(false);
    knob.current?.focus();
  };
  const toggle = (on: boolean) => {
    if (on && !settings.consentAck) setAsking(true);
    else sendRecording({ action: 'recording.enable', on });
  };
  return (
    <>
      <SettingsRow
        htmlFor={id}
        label="Record calls"
        description={!open ? binding : undefined}
        control={
          <Switch
            ref={knob}
            id={id}
            data-recording-knob
            checked={settings.enabled}
            aria-expanded={open || undefined}
            onCheckedChange={toggle}
          />
        }
      />
      {open && (
        <div
          className="rec-consent"
          data-recording-consent
          role="group"
          aria-labelledby={`${id}-consent`}
        >
          <p className="rec-consent-text" id={`${id}-consent`}>
            {RECORDING_TEXT.consent}
          </p>
          <p className="rec-hint">{binding}</p>
          <div className="hub-actions">
            <button
              ref={ok}
              type="button"
              className="hub-primary"
              onClick={() => {
                // One atomic ack: the brain saves the consent and turns the knob on.
                sendRecording({ action: 'recording.consent_ack' });
                close();
              }}
            >
              Got it
            </button>
            <button type="button" onClick={close}>
              Not now
            </button>
          </div>
        </div>
      )}
    </>
  );
}

function Length({ minutes }: { minutes: number }) {
  const [custom, setCustom] = useState(!isPreset(minutes));
  const [text, setText] = useState(isPreset(minutes) ? '' : String(minutes));
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const n = Number(text);
    if (text.trim() !== '') sendRecording({ action: 'recording.length', minutes: n });
  };
  return (
    <form className="rec-length" onSubmit={submit}>
      <div className="rec-segments" role="radiogroup" aria-labelledby="recording-length-label">
        {LENGTH_CHOICES.map((m) => (
          <label key={m} className="rec-segment">
            <input
              type="radio"
              name="recording-length"
              value={String(m)}
              checked={!custom && minutes === m}
              onChange={() => {
                setCustom(false);
                sendRecording({ action: 'recording.length', minutes: m });
              }}
            />
            <span>{m}</span>
          </label>
        ))}
        <label className="rec-segment">
          <input
            type="radio"
            name="recording-length"
            value="custom"
            checked={custom}
            onChange={() => setCustom(true)}
          />
          <span>Custom</span>
        </label>
      </div>
      {custom ? (
        <span className="rec-custom">
          <input
            type="number"
            inputMode="numeric"
            min={LENGTH_MIN}
            max={LENGTH_MAX}
            step={1}
            aria-label="Custom length in minutes"
            data-recording-custom-length
            value={text}
            placeholder="60"
            onChange={(e) => setText(e.currentTarget.value)}
            onBlur={(e) => e.currentTarget.form?.requestSubmit()}
          />
          <span className="rec-hint">
            min, {LENGTH_MIN} to {LENGTH_MAX}
          </span>
        </span>
      ) : (
        <span className="rec-hint">min</span>
      )}
    </form>
  );
}

function Storage({ storage }: { storage: StorageView }) {
  const [text, setText] = useState('');
  const [confirm, setConfirm] = useState<{ catalog: string; schema: string } | undefined>();
  const [changing, setChanging] = useState(false);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const v = text.trim();
    const m = CATALOG_SCHEMA.exec(v);
    if (m) setConfirm({ catalog: m[1] as string, schema: m[2] as string });
    else {
      setConfirm(undefined);
      sendRecording({ action: 'recording.storage_volume', path: v });
    }
  };
  const picker = storage.status === 'unavailable' || (storage.status === 'volume' && changing);
  return (
    <div className="rec-storage" data-recording-storage>
      {storage.status === 'checking' && (
        <p className="rec-hint" aria-busy="true">
          Checking My Files…
        </p>
      )}
      {storage.status === 'creating' && (
        <p className="rec-hint" aria-busy="true">
          Creating the volume bottle_audio…
        </p>
      )}
      {storage.status === 'my_files' && (
        <p className="rec-place">
          <span>My Files</span>
          {/* title (ui-1, review of #602 round 4, MEDIUM): .rec-path (hub.css) truncates a long
              path with an ellipsis instead of wrapping the row; the title attribute is the
              native way to still read the full path, on hover or with a screen reader. */}
          <span className="rec-path" title={storage.path}>
            {storage.path}
          </span>
        </p>
      )}
      {storage.status === 'volume' && !changing && (
        <p className="rec-place">
          <span>Volume</span>
          <span className="rec-path" title={storage.path}>
            {storage.path}
          </span>
          <button type="button" className="rec-link" onClick={() => setChanging(true)}>
            Change
          </button>
        </p>
      )}
      {picker && (
        <>
          {storage.status === 'unavailable' && (
            <p className="rec-hint">{RECORDING_TEXT.myFilesMissing}</p>
          )}
          <form className="rec-inline" onSubmit={submit}>
            <input
              aria-label="Volume path, or catalog and schema"
              data-recording-storage-input
              value={text}
              placeholder="/Volumes/catalog/schema/volume or catalog.schema"
              spellCheck={false}
              autoCapitalize="off"
              onChange={(e) => {
                setText(e.currentTarget.value);
                setConfirm(undefined);
              }}
            />
            <button type="submit" disabled={text.trim() === ''}>
              Use
            </button>
          </form>
          {confirm && (
            <div className="rec-confirm">
              <p className="rec-hint">
                Bottle creates a new managed volume in {confirm.catalog}.{confirm.schema}.
              </p>
              <div className="hub-actions">
                <button
                  type="button"
                  className="hub-primary"
                  data-recording-volume-create
                  onClick={() => {
                    sendRecording({ action: 'recording.volume_create', ...confirm });
                    setConfirm(undefined);
                    setChanging(false);
                  }}
                >
                  Create volume bottle_audio
                </button>
                <button type="button" onClick={() => setConfirm(undefined)}>
                  Cancel
                </button>
              </div>
            </div>
          )}
        </>
      )}
      {storage.status === 'unavailable' && storage.error && (
        <p className="hub-error" role="alert">
          {storage.error}
        </p>
      )}
    </div>
  );
}

function Endpoint({ which, endpoint }: { which: 'transcribe' | 'notes'; endpoint: string }) {
  const [text, setText] = useState(endpoint);
  const commit = () => {
    if (text.trim() !== endpoint)
      sendRecording({ action: 'recording.endpoint', which, endpoint: text });
  };
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        commit();
      }}
    >
      <input
        id={`recording-endpoint-${which}`}
        data-recording-endpoint={which}
        className="rec-wide"
        value={text}
        spellCheck={false}
        autoCapitalize="off"
        onChange={(e) => setText(e.currentTarget.value)}
        onBlur={commit}
      />
    </form>
  );
}

function Instructions({ instructions }: { instructions: string }) {
  const [text, setText] = useState(instructions);
  const isDefault = instructions === DEFAULT_INSTRUCTIONS;
  return (
    <div className="rec-instructions">
      <textarea
        id="recording-instructions"
        data-recording-instructions
        rows={6}
        value={text}
        onChange={(e) => setText(e.currentTarget.value)}
        onBlur={() => {
          if (text !== instructions) sendRecording({ action: 'recording.instructions', text });
        }}
      />
      <p className="rec-under">
        <span className="rec-hint">
          Bottle sends these with each transcript to the notes model.
        </span>
        <button
          type="button"
          className="rec-link"
          data-recording-instructions-reset
          disabled={isDefault && text === DEFAULT_INSTRUCTIONS}
          onClick={() => {
            setText(DEFAULT_INSTRUCTIONS);
            sendRecording({ action: 'recording.instructions', text: DEFAULT_INSTRUCTIONS });
          }}
        >
          Reset to default
        </button>
      </p>
    </div>
  );
}

export function RecordingSettings({ model }: { model: HubRecording | undefined }) {
  const s = model?.settings;
  return (
    <section
      data-view="recording-settings"
      aria-labelledby="recording-settings-title"
      className="hub-section rec-settings"
      id="recording-settings"
    >
      <h2 id="recording-settings-title" tabIndex={-1}>
        Call recording
      </h2>
      {!model || !s ? (
        <div className="rec-skeleton" aria-busy="true" aria-label="Loading" />
      ) : (
        <>
          {/* The whole section is one grouped box now (ui-1, review of #602 rounds 3-4, P1 and
              LOW): knob, length and storage as `SettingsRow`s, like `MicPreference`'s own
              `SettingsGroup`/`SettingsRow`; Instructions and Advanced moved in too (round 4 LOW)
              instead of sitting below the group as a second, differently-styled block. The
              consent notice (inside `Knob`) still renders as its own panel right under the
              knob's row, inside the group, when it shows. */}
          <SettingsGroup className="mt-[var(--mog-space-3)]">
            <Knob model={model} settings={s} />
            <SettingsRow
              // The id (not a plain string label) is `Length`'s `aria-labelledby` target
              // (`recording-length-label`, below): the radiogroup names itself from this row's
              // own visible label, not a separate hidden one.
              label={<span id="recording-length-label">Stops after</span>}
              control={<Length key={s.lengthMin} minutes={s.lengthMin} />}
            />
            <SettingsRow
              label="Save to"
              // min-w-0 flex-1 shrink (ui-1, review of #602 round 4, MEDIUM): a realistic path
              // is the one thing in this group that can run long, so it is the one control that
              // must give way instead of the "Save to" label. `.rec-path` (hub.css) truncates
              // the text itself with an ellipsis once it has room to do so.
              controlClassName="min-w-0 flex-1 shrink"
              control={<Storage storage={model.storage} />}
            />
            <div className="flex flex-col gap-[var(--mog-space-2)] px-[var(--mog-space-4)] py-[var(--mog-space-3)]">
              <label className="text-[var(--mog-text-md)]" htmlFor="recording-instructions">
                Instructions
              </label>
              <Instructions key={s.instructions} instructions={s.instructions} />
            </div>
            {/* ADR 0103 update 4: the two serving endpoints; most people never change them. One
                consistent "show more" style (ui-1, review of #602 round 3, P2): the same
                Collapsible, left chevron and text-sm/muted-foreground trigger as every other
                disclosure in Settings (Wipe, the Databricks CLI, the data panel, About), not a
                plain `<details>`/`<summary>` with its own hand-drawn chevron on the right. */}
            <div className="px-[var(--mog-space-4)] py-[var(--mog-space-3)]">
              <Collapsible data-recording-advanced>
                <CollapsibleTrigger className="group flex h-auto min-h-0 items-center gap-[var(--mog-space-2)] border-0 bg-transparent p-0 text-[var(--mog-text-sm)] text-muted-foreground hover:text-foreground">
                  <ChevronDownIcon className="size-3.5 transition-transform motion-reduce:transition-none group-data-[state=open]:rotate-180" />
                  Advanced
                </CollapsibleTrigger>
                <CollapsibleContent
                  forceMount
                  className="mt-[var(--mog-space-2)] data-[state=closed]:hidden"
                >
                  <div className="rec-fields rec-fields--inner">
                    <label className="rec-field-label" htmlFor="recording-endpoint-transcribe">
                      Transcribe model
                    </label>
                    <Endpoint
                      key={`t-${s.transcribeEndpoint}`}
                      which="transcribe"
                      endpoint={s.transcribeEndpoint}
                    />
                    <label className="rec-field-label" htmlFor="recording-endpoint-notes">
                      Notes model
                    </label>
                    <Endpoint
                      key={`n-${s.notesEndpoint}`}
                      which="notes"
                      endpoint={s.notesEndpoint}
                    />
                  </div>
                </CollapsibleContent>
              </Collapsible>
            </div>
          </SettingsGroup>
          {model.settingsError && (
            <p className="hub-error" role="alert">
              {model.settingsError}
            </p>
          )}
        </>
      )}
    </section>
  );
}
