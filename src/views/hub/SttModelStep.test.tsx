// @vitest-environment jsdom
import { act } from 'react';
import { render, tauri } from '../test-tauri';
import { Hub } from './Hub';
import {
  STT_ERROR_CODES,
  SttModelStep,
  sttErrorText,
  type SttModelApi,
  type SttModelView,
  type SttProgressView,
} from './SttModelStep';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const TOTAL = 487_211_443;

/** A test double of the `stt_model_*` commands (#529). Each install waits for the test. */
function fakeApi(start: Partial<SttModelView> = {}) {
  const view: SttModelView = {
    model: 'missing',
    partialBytes: 0,
    totalBytes: TOTAL,
    consent: 'not_asked',
    installing: false,
    ...start,
  };
  let pending: {
    resolve: (v: SttModelView) => void;
    reject: (e: unknown) => void;
    onProgress: (p: SttProgressView) => void;
  } | null = null;
  const calls: string[] = [];
  const api: SttModelApi = {
    status: async () => {
      calls.push('status');
      return { ...view };
    },
    setConsent: async (c) => {
      calls.push(`consent:${c}`);
      view.consent = c;
      return { ...view };
    },
    install: (source, onProgress) => {
      calls.push(`install:${source}`);
      view.installing = true;
      return new Promise((resolve, reject) => {
        pending = { resolve, reject, onProgress };
      });
    },
    verify: () => {
      calls.push('verify');
      view.installing = true;
      return new Promise((resolve, reject) => {
        pending = { resolve, reject, onProgress: () => undefined };
      });
    },
    cancel: async () => {
      calls.push('cancel');
      return view.installing;
    },
  };
  const finish = async (patch: Partial<SttModelView>) => {
    view.installing = false;
    Object.assign(view, patch);
    await act(async () => pending?.resolve({ ...view }));
  };
  const fail = async (code: string, patch: Partial<SttModelView> = {}) => {
    view.installing = false;
    Object.assign(view, patch);
    await act(async () => pending?.reject({ error: code }));
  };
  const progress = async (p: SttProgressView) => {
    await act(async () => pending?.onProgress(p));
  };
  return { api, calls, view, finish, fail, progress };
}

const step = (el: HTMLElement) => el.querySelector('[data-wizard-step="6"]') as HTMLElement;
const button = (el: HTMLElement, text: string) =>
  [...el.querySelectorAll('button')].find((b) => b.textContent === text);
const click = async (b: HTMLElement | undefined) => {
  expect(b).toBeDefined();
  await act(async () => b?.click());
};

describe('Wizard STT step (MB-07, #413)', () => {
  beforeEach(() => tauri.reset());

  it('[MB-07] [CAP-49a] asks for consent to the 487 MB download and starts nothing first', async () => {
    const f = fakeApi();
    const { el } = await render(<SttModelStep api={f.api} />);
    const s = step(el);
    expect(s).not.toBeNull();
    expect(s.textContent).toContain('487 MB');
    expect(s.textContent).toContain('Not downloaded');
    expect(button(s, 'Download (487 MB)')).toBeDefined();
    expect(button(s, 'Not now')).toBeDefined();
    expect(f.calls).toEqual(['status']);
  });

  it('[CAP-49a] "Not now" stores Declined and starts no install', async () => {
    const f = fakeApi();
    const { el } = await render(<SttModelStep api={f.api} />);
    await click(button(el, 'Not now'));
    expect(f.calls).toEqual(['status', 'consent:declined']);
    expect(step(el).textContent).toContain('Local speech is off');
    // The user can still agree later.
    expect(button(el, 'Download (487 MB)')).toBeDefined();
  });

  it('[CAP-49a] [MB-07] the download stores Granted before the install starts', async () => {
    const f = fakeApi();
    const done = vi.fn();
    const { el } = await render(<SttModelStep api={f.api} onDone={done} />);
    await click(button(el, 'Download (487 MB)'));
    expect(f.calls).toEqual(['status', 'consent:granted', 'install:github']);
    await f.finish({ model: 'installed' });
    expect(step(el).textContent).toContain('installed and checked');
    expect(step(el).getAttribute('data-done')).toBe('true');
    expect(done).toHaveBeenCalledTimes(1);
  });

  it('[CAP-49b] the user can pick the Hugging Face mirror as the source', async () => {
    const f = fakeApi();
    const { el } = await render(<SttModelStep api={f.api} />);
    const hf = el.querySelector<HTMLInputElement>('input[name="sttSource"][value="hf_mirror"]');
    await click(hf ?? undefined);
    await click(button(el, 'Download (487 MB)'));
    expect(f.calls).toContain('install:hf_mirror');
  });

  it('[CAP-49b] shows overall progress (no raw file name) and a Cancel button while the download runs', async () => {
    const f = fakeApi({ consent: 'granted' });
    const { el } = await render(<SttModelStep api={f.api} />);
    await click(button(el, 'Download (487 MB)'));
    await f.progress({ file: 'encoder.int8.onnx', done: 326_141_147, total: 652_282_294 });
    const bar = el.querySelector('[role="progressbar"]');
    expect(bar?.getAttribute('aria-valuenow')).toBe('50');
    expect(step(el).textContent).toContain('326 MB of 652 MB');
    expect(step(el).textContent).not.toContain('encoder.int8.onnx');
    await click(button(el, 'Cancel'));
    expect(f.calls.at(-1)).toBe('cancel');
  });

  it('[#413 review] adds progress across files instead of restarting the bar per file', async () => {
    const f = fakeApi({ consent: 'granted' });
    const { el } = await render(<SttModelStep api={f.api} />);
    const hf = el.querySelector<HTMLInputElement>('input[name="sttSource"][value="hf_mirror"]');
    await click(hf ?? undefined);
    await click(button(el, 'Download (487 MB)'));
    await f.progress({ file: 'encoder.int8.onnx', done: 652_282_294, total: 652_282_294 });
    await f.progress({ file: 'decoder.int8.onnx', done: 5_000_000, total: 11_845_274 });
    const bar = el.querySelector('[role="progressbar"]');
    // Overall: 652,282,294 (encoder, done) + 5,000,000 of 11,845,274 (decoder, in flight).
    expect(step(el).textContent).toContain('657 MB of 664 MB');
    expect(bar?.getAttribute('aria-valuenow')).toBe('98');
  });

  it('[#413 review] shows "Unpacking…" instead of a stuck 100% bar, and hides Cancel', async () => {
    const f = fakeApi({ consent: 'granted' });
    const { el } = await render(<SttModelStep api={f.api} />);
    await click(button(el, 'Download (487 MB)'));
    await f.progress({
      file: 'sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2',
      done: 487_211_443,
      total: 487_211_443,
    });
    expect(step(el).textContent).toContain('Unpacking');
    expect(el.querySelector('[role="progressbar"]')).toBeNull();
    expect(button(el, 'Cancel')).toBeUndefined();
  });

  it('[CAP-49b] after a cancel the step shows Partial and a retry resumes', async () => {
    const f = fakeApi({ consent: 'granted' });
    const { el } = await render(<SttModelStep api={f.api} />);
    await click(button(el, 'Download (487 MB)'));
    await click(button(el, 'Cancel'));
    await f.fail('stt_download_cancelled', { model: 'partial', partialBytes: 120_000_000 });
    const s = step(el);
    expect(s.textContent).toContain(sttErrorText('stt_download_cancelled'));
    expect(s.textContent).toContain('Partly downloaded: 120 MB of 487 MB');
    await click(button(el, 'Retry'));
    expect(f.calls.filter((c) => c.startsWith('install:'))).toHaveLength(2);
    await f.finish({ model: 'installed', partialBytes: 0 });
    expect(step(el).getAttribute('data-done')).toBe('true');
  });

  it('[CAP-49c] a hash mismatch says the install stopped', async () => {
    const f = fakeApi({ consent: 'granted' });
    const { el } = await render(<SttModelStep api={f.api} />);
    await click(button(el, 'Download (487 MB)'));
    await f.fail('stt_hash_mismatch');
    const alert = el.querySelector('[role="alert"]');
    expect(alert?.textContent).toBe(sttErrorText('stt_hash_mismatch'));
    expect(alert?.textContent).toMatch(/install stopped/i);
    expect(step(el).getAttribute('data-done')).toBe('false');
    expect(button(el, 'Retry')).toBeDefined();
  });

  it('[CAP-49c] every error code has its own user text and no code shows raw', () => {
    const texts = STT_ERROR_CODES.map(sttErrorText);
    expect(new Set(texts).size).toBe(STT_ERROR_CODES.length);
    for (const [i, t] of texts.entries()) {
      expect(t).not.toContain(STT_ERROR_CODES[i]);
      expect(t.length).toBeGreaterThan(10);
    }
    expect(sttErrorText('something_new')).toMatch(/install stopped/i);
  });

  it('[MB-07] an installed model is checked before the step is done', async () => {
    const f = fakeApi({ consent: 'granted', model: 'installed' });
    const { el } = await render(<SttModelStep api={f.api} />);
    expect(step(el).textContent).toContain('Checking the speech model');
    expect(step(el).getAttribute('data-done')).toBe('false');
    expect(button(el, 'Cancel')).toBeUndefined();
    expect(f.calls).toEqual(['status', 'verify']);
    await f.finish({});
    expect(step(el).getAttribute('data-done')).toBe('true');
  });

  it('[#413 review] an installed model with no stored consent is checked, never asked, and ends in one calm state', async () => {
    const f = fakeApi({ consent: 'not_asked', model: 'installed' });
    const { el } = await render(<SttModelStep api={f.api} />);
    // The real state on a Mac that already has the model: no `stt-download-consent` file.
    // `verify` sends no request, so it must run regardless, with no consent question first.
    expect(f.calls).toEqual(['status', 'verify']);
    expect(step(el).textContent).toContain('Checking the speech model');
    expect(button(el, 'Download (487 MB)')).toBeUndefined();
    expect(button(el, 'Not now')).toBeUndefined();
    expect(el.querySelector('input[name="sttSource"]')).toBeNull();
    await f.finish({});
    expect(step(el).getAttribute('data-done')).toBe('true');
    expect(step(el).textContent).toContain('Speech model ready');
    // One calm state: no download button, no consent ask, no source picker, and the stale
    // "Downloaded" status line is not shown alongside it.
    expect(button(el, 'Download (487 MB)')).toBeUndefined();
    expect(button(el, 'Not now')).toBeUndefined();
    expect(el.querySelector('input[name="sttSource"]')).toBeNull();
    expect(step(el).textContent?.match(/Downloaded/g) ?? []).toHaveLength(0);
  });

  it('[CAP-49a] [CAP-49c] a failed launch check downloads nothing before a click', async () => {
    const f = fakeApi({ consent: 'granted', model: 'installed' });
    const { el } = await render(<SttModelStep api={f.api} />);
    await f.fail('stt_hash_mismatch', { model: 'missing' });
    expect(el.querySelector('[role="alert"]')?.textContent).toBe(sttErrorText('stt_hash_mismatch'));
    expect(step(el).textContent).toContain('Not downloaded');
    expect(f.calls.filter((c) => c.startsWith('install:'))).toEqual([]);
    await click(button(el, 'Retry'));
    expect(f.calls.filter((c) => c.startsWith('install:'))).toEqual(['install:github']);
  });

  it('[MB-07] the hub shows the STT step and sends no stt command before a click', async () => {
    const { el } = await render(<Hub />);
    expect(step(el)).not.toBeNull();
    const calls = tauri.invoke.mock.calls.map(([cmd]) => String(cmd));
    expect(calls.filter((c) => c.startsWith('stt_model_install'))).toEqual([]);
  });
});
