/**
 * Settings → Call recording in the hub (thin; ADR 0103, refs #65, ARC-2).
 *
 * It renders the brain model (`hub:recording`) and sends each change with `view_emit` (id
 * `recording`). The brain checks every value and owns every text; the core checks the knob again
 * before any capture starts. The first turn-on shows the consent notice once, with one
 * acknowledgement. The view never builds a path or a URL; it only sends what the user typed.
 */
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
import { sendRecording } from './recording-ipc';

const isPreset = (m: number) => (LENGTH_CHOICES as readonly number[]).includes(m);

function Knob({ model, settings }: { model: HubRecording; settings: RecordingSettingsView }) {
  const [asking, setAsking] = useState(false);
  const id = useId();
  const knob = useRef<HTMLInputElement>(null);
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
      <div className="rec-knob">
        <label htmlFor={id} className="rec-knob-label">
          <span>Record calls</span>
          {!open && <span className="rec-hint">{binding}</span>}
        </label>
        <input
          ref={knob}
          id={id}
          type="checkbox"
          role="switch"
          className="hub-switch"
          data-recording-knob
          checked={settings.enabled}
          aria-expanded={open || undefined}
          onChange={(e) => toggle(e.currentTarget.checked)}
        />
      </div>
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
          <span className="rec-path">{storage.path}</span>
        </p>
      )}
      {storage.status === 'volume' && !changing && (
        <p className="rec-place">
          <span>Volume</span>
          <span className="rec-path">{storage.path}</span>
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
          <Knob model={model} settings={s} />
          {model.settingsError && (
            <p className="hub-error" role="alert">
              {model.settingsError}
            </p>
          )}
          <div className="rec-fields">
            <span className="rec-field-label" id="recording-length-label">
              Stops after
            </span>
            <Length key={s.lengthMin} minutes={s.lengthMin} />

            <span className="rec-field-label">Save to</span>
            <Storage storage={model.storage} />

            <label
              className="rec-field-label rec-field-label--top"
              htmlFor="recording-instructions"
            >
              Instructions
            </label>
            <Instructions key={s.instructions} instructions={s.instructions} />
          </div>
          {/* ADR 0103 update 4: the two serving endpoints; most people never change them. */}
          <details className="hub-disclosure rec-advanced" data-recording-advanced>
            <summary>Advanced</summary>
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
              <Endpoint key={`n-${s.notesEndpoint}`} which="notes" endpoint={s.notesEndpoint} />
            </div>
          </details>
        </>
      )}
    </section>
  );
}
