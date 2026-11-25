/**
 * The Record control and the REC indicator in the hub (thin; ADR 0103, refs #65, ARC-2).
 *
 * Record sends `recording.start`, with instructions only when the user edited them for this
 * recording. The global preset in Settings is what most recordings use. While a recording runs,
 * the indicator shows REC and the time left to `ends_at_ms`, with Stop. An error shows its
 * plain text and one action; "Open Settings" moves the hub to Settings, the others go to the
 * brain.
 */
import { useId, useState } from 'react';
import {
  DEFAULT_INSTRUCTIONS,
  RECORDING_TEXT,
  timeLeftOf,
  type HubRecording,
  type RecordingNotice,
} from '../../shared/recording-view';
import { sendRecording, useNow } from './recording-ipc';

/** One plain-words problem and its one action. */
export function RecordingAlert({
  notice,
  onSettings,
  onRetry,
  onOpen,
}: {
  notice: RecordingNotice;
  onSettings?: (() => void) | undefined;
  onRetry?: () => void;
  onOpen?: () => void;
}) {
  const act = notice.action;
  const run = () => {
    switch (act?.target) {
      case 'settings':
        onSettings?.();
        break;
      case 'system_audio':
        sendRecording({ action: 'recording.system_audio' });
        break;
      case 'retry':
        onRetry?.();
        break;
      case 'open':
        onOpen?.();
        break;
      default:
        break;
    }
  };
  const can =
    act !== undefined &&
    (act.target === 'system_audio' ||
      (act.target === 'settings' && onSettings !== undefined) ||
      (act.target === 'retry' && onRetry !== undefined) ||
      (act.target === 'open' && onOpen !== undefined));
  // A calm hint (CAP-10a review, #593 follow-up) reads as status, not an alarm.
  const role = notice.role === 'status' ? 'status' : 'alert';
  return (
    <div className={role === 'status' ? 'rec-alert rec-alert--status' : 'rec-alert'} role={role}>
      <p>{notice.text}</p>
      {can && (
        <button type="button" onClick={run}>
          {act.label}
        </button>
      )}
    </div>
  );
}

/** REC, the time left and Stop. */
export function RecIndicator({
  endsAtMs,
  stopping,
  now = Date.now,
  compact = false,
}: {
  endsAtMs: number | undefined;
  stopping: boolean;
  now?: () => number;
  compact?: boolean;
}) {
  const t = useNow(!stopping, now);
  const left = endsAtMs === undefined ? undefined : timeLeftOf(endsAtMs, t);
  return (
    <div className={compact ? 'rec-on rec-on--compact' : 'rec-on'} data-recording-indicator>
      <span className="rec-dot" aria-hidden="true" />
      <span className="rec-on-label">REC</span>
      {stopping ? (
        <span className="rec-on-left">Stopping…</span>
      ) : (
        left !== undefined && (
          // The time ticks each second; screen readers get it on demand, not on each tick.
          <span className="rec-on-left" aria-live="off">
            {left} left
          </span>
        )
      )}
      <button
        type="button"
        className="rec-stop"
        data-recording-stop
        disabled={stopping}
        onClick={() => sendRecording({ action: 'recording.stop' })}
      >
        Stop
      </button>
    </div>
  );
}

export function RecordControl({
  model,
  now,
  onOpenSettings,
}: {
  model: HubRecording | undefined;
  now?: () => number;
  onOpenSettings?: () => void;
}) {
  const preset = model?.settings?.instructions ?? DEFAULT_INSTRUCTIONS;
  const [text, setText] = useState<string | undefined>(undefined);
  const [editing, setEditing] = useState(false);
  const editId = useId();
  const enabled = model?.settings?.enabled === true;
  const phase = model?.control.phase ?? 'idle';
  const live = phase === 'recording' || phase === 'stopping';

  return (
    <div className="rec-control" data-recording-control>
      {live ? (
        <RecIndicator
          endsAtMs={model?.control.endsAtMs}
          stopping={phase === 'stopping'}
          {...(now ? { now } : {})}
        />
      ) : (
        <div className="rec-start-row">
          <button
            type="button"
            className="hub-primary rec-start"
            data-recording-start
            disabled={!enabled || phase === 'starting'}
            onClick={() => {
              const edited = text !== undefined && text.trim() !== preset.trim();
              sendRecording(
                edited
                  ? { action: 'recording.start', instructions: text }
                  : { action: 'recording.start' },
              );
            }}
          >
            <span className="rec-dot" aria-hidden="true" />
            {phase === 'starting' ? 'Starting…' : 'Record'}
          </button>
          {model && !enabled && (
            <p className="rec-hint">
              {RECORDING_TEXT.turnOn}{' '}
              {onOpenSettings && (
                <button type="button" className="rec-link" onClick={onOpenSettings}>
                  Open Settings
                </button>
              )}
            </p>
          )}
          {model && enabled && (
            <p className="rec-hint">
              {RECORDING_TEXT.binding[model.platform]}
              <span aria-hidden="true"> · </span>
              <button
                type="button"
                className="rec-link"
                data-recording-edit-toggle
                aria-expanded={editing}
                aria-controls={editId}
                onClick={() => setEditing((v) => !v)}
              >
                Edit instructions for this recording
              </button>
            </p>
          )}
        </div>
      )}
      {!live && enabled && editing && (
        <div className="rec-edit" id={editId} data-recording-edit>
          <textarea
            aria-label="Instructions for this recording"
            rows={7}
            value={text ?? preset}
            onChange={(e) => setText(e.currentTarget.value)}
          />
          <p className="rec-hint">Only this recording uses these. Settings keeps the preset.</p>
        </div>
      )}
      {model?.error && <RecordingAlert notice={model.error} onSettings={onOpenSettings} />}
    </div>
  );
}
