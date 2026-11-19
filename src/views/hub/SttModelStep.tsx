/**
 * `SttModelStep`: "Local speech" (MB-07, #413, #45, #529, ADR 0049, ADR 0104), one
 * self-contained component. It asks consent for the 487 MB Parakeet model (CAP-49a), shows the
 * model status, download progress, Cancel and a Retry that resumes (CAP-49b), and the
 * content-free error codes as user text (CAP-49c). It renders its own `<section>` and assumes
 * nothing about where it is mounted, so the hub can place it in the first-run Setup region, or
 * in Settings > Local speech once the step is no longer first-run (ADR 0104).
 *
 * The step stores the answer before any install starts. The core gate reads that stored answer,
 * so no download starts without Granted. `status` checks file sizes only, so the step is done
 * only after an `install` call (which checks every SHA-256) resolves. `sttModelApi` (same
 * folder) is the real `api`, over the `stt_model_*` commands of #529; the hub passes it in.
 *
 * TODO(#43): the wizard frame may also mount this step directly for a first-run flow.
 */
import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';

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

/** One chunk of download progress, keyed by file name, so a multi-file source (the Hugging
 * Face mirror) adds up across files instead of a bar that restarts at each one (#413 review). */
type FileTotals = ReadonlyMap<string, { done: number; total: number }>;

function overallBytes(files: FileTotals): { done: number; total: number } {
  let done = 0;
  let total = 0;
  for (const f of files.values()) {
    done += f.done;
    total += f.total;
  }
  return { done, total };
}

export function SttModelStep({
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
  // Keyed by file name and summed in render (`overallBytes`), so a multi-file source (the
  // Hugging Face mirror) adds up across files instead of a bar that restarts at each one, and
  // never shows a raw file name (#413 review).
  const [files, setFiles] = useState<FileTotals>(new Map());
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
      setFiles(new Map());
      try {
        const v = check
          ? await api.verify()
          : await api.install(from, (p) => {
              if (!live.current) return;
              setFiles((prev) => new Map(prev).set(p.file, { done: p.done, total: p.total }));
            });
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
          setFiles(new Map());
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
        // #413 review: an installed model is checked every time, whatever the stored consent.
        // `verify` sends no request, so no consent gate applies; only a download needs consent
        // (CAP-49a). Sizes alone do not prove the model: the check hashes every file, and after
        // a mismatch only a click starts a download.
        if (v.model === 'installed') void run('github', true);
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
  const total = view?.totalBytes ?? 487_211_443;
  // Overall bytes across every file seen this run, not the raw file name of whichever one is
  // downloading now: a multi-file source (the Hugging Face mirror) would otherwise show a bar
  // that restarts at each file (#413 review).
  const { done: overallDone, total: overallTotal } = overallBytes(files);
  const percent = overallTotal > 0 ? Math.floor((overallDone * 100) / overallTotal) : 0;
  // Every byte of every file seen so far has arrived, but the install has not resolved: it is
  // extracting the archive and hashing the result, with no progress event for that work. A
  // stuck 100% bar with a Cancel that cannot stop it is confusing, so show a calm phase message
  // instead (#413 review).
  const unpacking = installing && !checking && overallTotal > 0 && overallDone >= overallTotal;

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
      {view && !done && (
        <p data-stt-status={view.model} className="hub-status">
          {statusText(view)}
        </p>
      )}
      {view?.consent === 'declined' && !installing && !done && (
        <p>Local speech is off. You can download the model at any time.</p>
      )}
      {!installing && !done && view && (
        <fieldset className="hub-radios">
          <legend>Download source</legend>
          <input
            id="stt-source-github"
            type="radio"
            name="sttSource"
            value="github"
            checked={source === 'github'}
            onChange={() => setSource('github')}
          />
          <label htmlFor="stt-source-github">sherpa-onnx release on GitHub</label>
          <input
            id="stt-source-hf"
            type="radio"
            name="sttSource"
            value="hf_mirror"
            checked={source === 'hf_mirror'}
            onChange={() => setSource('hf_mirror')}
          />
          <label htmlFor="stt-source-hf">Hugging Face mirror</label>
        </fieldset>
      )}
      {checking && <p>Checking the speech model…</p>}
      {installing && files.size > 0 && !unpacking && (
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
            {mb(overallDone)} of {mb(overallTotal)}
          </p>
        </div>
      )}
      {unpacking && <p className="hub-muted">Unpacking…</p>}
      {installing && !checking && !unpacking && (
        <button type="button" onClick={() => void api.cancel()}>
          Cancel
        </button>
      )}
      {error && (
        <p role="alert" className="hub-error">
          {sttErrorText(error)}
        </p>
      )}
      {done && (
        <p data-stt-status="installed" className="hub-status">
          Speech model ready. The model is installed and checked.
        </p>
      )}
      {!installing && !done && view && (
        <div className="hub-actions">
          {error && error !== 'stt_consent_required' && view.consent === 'granted' ? (
            <button type="button" className="hub-primary" onClick={() => void run(source)}>
              Retry
            </button>
          ) : (
            <button type="button" className="hub-primary" onClick={() => void start()}>
              Download ({mb(total)})
            </button>
          )}
          {view.consent === 'not_asked' && (
            <button type="button" onClick={() => void choose('declined')}>
              Not now
            </button>
          )}
        </div>
      )}
    </section>
  );
}
