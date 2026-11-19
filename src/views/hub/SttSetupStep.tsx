/**
 * Wizard step 6 "Local speech" (MB-07, #413, ADR 0049): the consent question for the 487 MB
 * Parakeet model (CAP-49a), the model status, progress, Cancel and a Retry that resumes
 * (CAP-49b), and the content-free error codes as user text (CAP-49c).
 *
 * The step stores the answer before any install starts. The core gate reads that stored answer,
 * so no download starts without Granted. `status` checks file sizes only, so the step is done
 * only after an `install` call (which checks every SHA-256) resolves.
 *
 * TODO(#529): the `stt_model_*` hub commands. Until they land, the hub uses
 * `unwiredSttModelApi`, which answers `stt_setup_unavailable`. TODO(#43): the wizard frame
 * mounts this step; until then the hub shows it on its own.
 *
 * ui-1 (ADR 0104): rebuilt on shadcn/ui's RadioGroup and Button instead of a raw
 * `<fieldset>`/`<legend>` with bare `<input type="radio">`s and plain `<button>`s (the ui-audit
 * "Settings baseline" findings). The RadioGroup renders as accessible buttons
 * (`role="radio"`/`aria-checked`), not native `<input>`s, so SttSetupStep.test.tsx asserts on
 * role, not `input[type="radio"]` (documented there).
 */
import { useCallback, useEffect, useId, useRef, useState, type CSSProperties } from 'react';
import { Button } from '../ui/button';
import { RadioGroup, RadioGroupItem } from '../ui/radio-group';

export type SttModelState = 'missing' | 'partial' | 'installed';
export type SttConsent = 'not_asked' | 'declined' | 'granted';
export type SttSource = 'github' | 'hf_mirror';

/** Mirrors `SttModelStatus` of #529. */
export interface SttModelView {
  model: SttModelState;
  partialBytes: number;
  totalBytes: number;
  consent: SttConsent;
  installing: boolean;
}

/** Mirrors `SttModelProgress` of #529: a pinned file name and byte counts only. */
export interface SttProgressView {
  file: string;
  done: number;
  total: number;
}

/** The `stt_model_*` commands (#529). A refusal rejects with `{ error: <code> }`. */
export interface SttModelApi {
  status(): Promise<SttModelView>;
  setConsent(consent: 'granted' | 'declined'): Promise<SttModelView>;
  install(source: SttSource, onProgress: (p: SttProgressView) => void): Promise<SttModelView>;
  /** The launch check: every SHA-256, no request. A mismatch deletes the model files. */
  verify(): Promise<SttModelView>;
  cancel(): Promise<boolean>;
}

/** The codes of `mog_core::stt_setup::SetupError::code`, plus the unwired seam. */
export const STT_ERROR_CODES = [
  'stt_consent_required',
  'stt_url_not_allowed',
  'stt_download_failed',
  'stt_hash_mismatch',
  'stt_extract_failed',
  'stt_io_failed',
  'stt_download_cancelled',
  'stt_install_busy',
  'stt_models_dir_unavailable',
  'stt_setup_unavailable',
] as const;
export type SttErrorCode = (typeof STT_ERROR_CODES)[number];

const ERROR_TEXT: Record<SttErrorCode, string> = {
  stt_consent_required: 'Bottle needs your agreement before it downloads the speech model.',
  stt_url_not_allowed:
    'The install stopped. The download address is not one of the approved sources.',
  stt_download_failed:
    'The download stopped. Check your network connection, then select Retry. The download continues at the point where it stopped.',
  stt_hash_mismatch:
    'The install stopped. A downloaded file did not match its pinned checksum, so Bottle deleted it. Select Retry to download it again.',
  stt_extract_failed: 'The install stopped. The model archive did not unpack. Select Retry.',
  stt_io_failed:
    'The install stopped. Bottle could not write to the model folder. Check the free disk space, then select Retry.',
  stt_download_cancelled:
    'You cancelled the download. Select Retry to continue at the point where it stopped.',
  stt_install_busy: 'A download of the speech model is already in progress.',
  stt_models_dir_unavailable:
    'The install stopped. Bottle cannot find your user folder for the speech model.',
  stt_setup_unavailable: 'Local speech setup is not available in this build yet.',
};

/** The user text for an error code. An unknown code gets a general text, never the code. */
export function sttErrorText(code: string): string {
  return (ERROR_TEXT as Record<string, string>)[code] ?? 'The install stopped. Select Retry.';
}

function errorCode(e: unknown): string {
  const code = (e as { error?: unknown } | null)?.error;
  return typeof code === 'string' ? code : 'unknown';
}

const mb = (bytes: number) => `${Math.round(bytes / 1_000_000)} MB`;

/** TODO(#529): replace with the `stt_model_*` commands. */
export const unwiredSttModelApi: SttModelApi = {
  status: () => Promise.reject({ error: 'stt_setup_unavailable' }),
  setConsent: () => Promise.reject({ error: 'stt_setup_unavailable' }),
  install: () => Promise.reject({ error: 'stt_setup_unavailable' }),
  verify: () => Promise.reject({ error: 'stt_setup_unavailable' }),
  cancel: () => Promise.resolve(false),
};

function statusText(v: SttModelView): string {
  switch (v.model) {
    case 'installed':
      return 'Downloaded';
    case 'partial':
      return `Partly downloaded: ${mb(v.partialBytes)} of ${mb(v.totalBytes)}. A retry continues at that point.`;
    default:
      return 'Not downloaded';
  }
}

export function SttSetupStep({
  api = unwiredSttModelApi,
  onDone,
}: {
  api?: SttModelApi;
  onDone?: () => void;
}) {
  const [view, setView] = useState<SttModelView | null>(null);
  const [source, setSource] = useState<SttSource>('github');
  const [installing, setInstalling] = useState(false);
  const [checking, setChecking] = useState(false);
  const [progress, setProgress] = useState<SttProgressView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);
  const live = useRef(true);
  const doneRef = useRef(onDone);
  useEffect(() => {
    doneRef.current = onDone;
  }, [onDone]);

  // `check`: the launch check (verify, no download); else an install from `from`.
  const run = useCallback(
    async (from: SttSource, check = false) => {
      setError(null);
      setInstalling(true);
      setChecking(check);
      setProgress(null);
      try {
        const v = check
          ? await api.verify()
          : await api.install(from, (p) => live.current && setProgress(p));
        if (!live.current) return;
        setView(v);
        setDone(true);
        doneRef.current?.();
      } catch (e) {
        if (!live.current) return;
        setError(errorCode(e));
        // The core status shows what the stopped download kept for a resume.
        const v = await api.status().catch(() => null);
        if (live.current && v) setView(v);
      } finally {
        if (live.current) {
          setInstalling(false);
          setChecking(false);
          setProgress(null);
        }
      }
    },
    [api],
  );

  useEffect(() => {
    live.current = true;
    void api.status().then(
      (v) => {
        if (!live.current) return;
        setView(v);
        // MB-07: sizes alone do not prove the model. The check hashes every file and never
        // downloads; after a mismatch only a click starts a download (CAP-49a).
        if (v.consent === 'granted' && v.model === 'installed') void run('github', true);
      },
      (e) => live.current && setError(errorCode(e)),
    );
    return () => {
      live.current = false;
    };
  }, [api, run]);

  const choose = async (answer: 'granted' | 'declined') => {
    setError(null);
    try {
      // CAP-49a: the answer is stored before the install starts.
      setView(await api.setConsent(answer));
    } catch (e) {
      setError(errorCode(e));
      return;
    }
    if (answer === 'granted') await run(source);
  };

  const start = () => (view?.consent === 'granted' ? run(source) : choose('granted'));
  const sourceLabelId = useId();
  const total = view?.totalBytes ?? 487_211_443;
  const percent =
    progress && progress.total > 0 ? Math.floor((progress.done * 100) / progress.total) : 0;

  return (
    <section
      data-view="stt-setup"
      data-wizard-step="6"
      data-done={String(done)}
      aria-labelledby="stt-setup-title"
      className="hub-section"
    >
      <h2 id="stt-setup-title">Local speech</h2>
      <p>
        Bottle turns your speech into text on this device. It needs a speech model of {mb(total)}.
        The model downloads only after you agree.
      </p>
      {view && (
        <p data-stt-status={view.model} className="hub-status">
          {statusText(view)}
        </p>
      )}
      {view?.consent === 'declined' && !installing && !done && (
        <p>Local speech is off. You can download the model at any time.</p>
      )}
      {!installing && !done && view && (
        <div className="mt-[var(--mog-space-3)]">
          <span
            id={sourceLabelId}
            className="text-[var(--mog-text-sm)] font-medium text-muted-foreground"
          >
            Download source
          </span>
          <RadioGroup
            aria-labelledby={sourceLabelId}
            className="mt-[var(--mog-space-2)] gap-[var(--mog-space-2)]"
            value={source}
            onValueChange={(v) => setSource(v as SttSource)}
          >
            <label
              htmlFor="stt-source-github"
              className="flex items-center gap-[var(--mog-space-2)] cursor-pointer"
            >
              <RadioGroupItem id="stt-source-github" value="github" />
              sherpa-onnx release on GitHub
            </label>
            <label
              htmlFor="stt-source-hf"
              className="flex items-center gap-[var(--mog-space-2)] cursor-pointer"
            >
              <RadioGroupItem id="stt-source-hf" value="hf_mirror" />
              Hugging Face mirror
            </label>
          </RadioGroup>
        </div>
      )}
      {checking && <p>Checking the speech model…</p>}
      {installing && progress && (
        <div>
          <div
            role="progressbar"
            className="hub-progress"
            // CSSOM, not a style attribute, so the CSP holds (no inline style element).
            style={{ '--hub-progress': `${percent}%` } as CSSProperties}
            aria-label="Speech model download"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent}
          />
          <p className="hub-muted">
            {progress.file}: {mb(progress.done)} of {mb(progress.total)}
          </p>
        </div>
      )}
      {installing && !checking && (
        <Button type="button" variant="outline" onClick={() => void api.cancel()}>
          Cancel
        </Button>
      )}
      {error && (
        // ui-1 (ADR 0104): `stt_setup_unavailable` is a missing-feature state, not an error — the
        // pink error colour (the lamp-lid pink) read as "broken" for a "not built yet" message
        // that fires by default with no brain connected (the ui-audit "contrast/semantics"
        // finding). Every other code keeps the error style; only this one gets the neutral note.
        <p role="alert" className={error === 'stt_setup_unavailable' ? 'hub-note' : 'hub-error'}>
          {sttErrorText(error)}
        </p>
      )}
      {done && <p>The speech model is installed and checked.</p>}
      {!installing && !done && view && (
        <div className="mt-[var(--mog-space-3)] flex flex-wrap gap-[var(--mog-space-2)]">
          {error && error !== 'stt_consent_required' && view.consent === 'granted' ? (
            <Button type="button" onClick={() => void run(source)}>
              Retry
            </Button>
          ) : (
            <Button type="button" onClick={() => void start()}>
              Download ({mb(total)})
            </Button>
          )}
          {view.consent === 'not_asked' && (
            <Button type="button" variant="outline" onClick={() => void choose('declined')}>
              Not now
            </Button>
          )}
        </div>
      )}
    </section>
  );
}
