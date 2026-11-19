/**
 * The first-run notice "Before you start" in the hub (9.3, MB-08a, MB-08b, ADR 0044).
 * It renders what the brain sends. Continue stays off until the user checks the box. The brain
 * decides when the notice is accepted; the view only sends the click.
 */
import { useEffect, useId, useRef, useState } from 'react';
import { LEGAL_NOTICE } from '../../shared/hub-notice';

export function LegalNotice({
  version,
  storeError = false,
  onContinue,
}: {
  version: number;
  /** The brain could not record the click (#541): show the fixed message only. */
  storeError?: boolean;
  onContinue: (version: number) => void;
}) {
  const [checked, setChecked] = useState(false);
  const ref = useRef<HTMLElement>(null);
  const id = useId();
  // The hub takes focus (ADR 0022 decision 3); the notice gets it first.
  useEffect(() => ref.current?.focus(), []);
  return (
    <section
      ref={ref}
      role="dialog"
      aria-modal="true"
      aria-labelledby={`${id}-title`}
      aria-describedby={`${id}-body`}
      tabIndex={-1}
      className="legal-notice"
    >
      <h2 id={`${id}-title`}>{LEGAL_NOTICE.title}</h2>
      <p className="hub-muted">Notice version {version}</p>
      {/* TODO(#394): the copy is a draft until legal or admin review approves it. */}
      {LEGAL_NOTICE.draft && (
        <p className="hub-draft">Draft notice. This text is not yet approved.</p>
      )}
      <div id={`${id}-body`} className="legal-notice-body">
        {LEGAL_NOTICE.paragraphs.map((p) => (
          <p key={p.id}>{p.text}</p>
        ))}
      </div>
      <label className="hub-check">
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => setChecked(e.currentTarget.checked)}
        />
        {LEGAL_NOTICE.checkboxLabel}
      </label>
      <button
        type="button"
        className="hub-primary"
        disabled={!checked}
        onClick={() => onContinue(version)}
      >
        {LEGAL_NOTICE.continueLabel}
      </button>
      {storeError && (
        <p role="alert" className="hub-error">
          {LEGAL_NOTICE.storeErrorText}
        </p>
      )}
    </section>
  );
}
