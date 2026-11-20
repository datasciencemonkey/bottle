// @vitest-environment jsdom
import { tauri } from '../test-tauri';
import { sttModelApi } from './sttModelApi';
import type { SttModelView } from './SttModelStep';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);

const view: SttModelView = {
  model: 'missing',
  partialBytes: 0,
  totalBytes: 487_211_443,
  consent: 'not_asked',
  installing: false,
};

type Args = Record<string, unknown>;
const argsOf = (n: number) => (tauri.invoke.mock.calls[n]?.[1] as { args: Args } | undefined)?.args;

describe('sttModelApi: the real stt_model_* adapter (#413, #45, #529, ADR 0104)', () => {
  beforeEach(() => tauri.reset());

  it('status() sends stt_model_status with no arguments', async () => {
    tauri.invoke.mockResolvedValueOnce(view);
    await expect(sttModelApi.status()).resolves.toEqual(view);
    expect(tauri.invoke.mock.calls[0]?.[0]).toBe('stt_model_status');
    expect(argsOf(0)).toEqual({});
  });

  it('setConsent() sends the chosen answer', async () => {
    tauri.invoke.mockResolvedValueOnce({ ...view, consent: 'granted' });
    await sttModelApi.setConsent('granted');
    expect(tauri.invoke.mock.calls[0]?.[0]).toBe('stt_model_consent');
    expect(argsOf(0)).toEqual({ consent: 'granted' });
  });

  it('verify() and cancel() send no arguments', async () => {
    tauri.invoke.mockResolvedValueOnce(view);
    await sttModelApi.verify();
    expect(tauri.invoke.mock.calls[0]?.[0]).toBe('stt_model_verify');
    expect(argsOf(0)).toEqual({});

    tauri.invoke.mockResolvedValueOnce({ cancelled: true });
    await expect(sttModelApi.cancel()).resolves.toBe(true);
    expect(tauri.invoke.mock.calls[1]?.[0]).toBe('stt_model_cancel');
  });

  it('install() sends the source and a progress Channel that relays to onProgress', async () => {
    tauri.invoke.mockResolvedValueOnce({ ...view, model: 'installed' });
    const chunks: unknown[] = [];
    const resultP = sttModelApi.install('hf_mirror', (p) => chunks.push(p));
    expect(tauri.invoke.mock.calls[0]?.[0]).toBe('stt_model_install');
    const args = argsOf(0);
    expect(args?.['source']).toBe('hf_mirror');
    const channel = args?.['onProgress'] as { onmessage?: (p: unknown) => void };
    expect(typeof channel.onmessage).toBe('function');
    channel.onmessage?.({ file: 'fake.bin', done: 10, total: 21 });
    expect(chunks).toEqual([{ file: 'fake.bin', done: 10, total: 21 }]);
    await expect(resultP).resolves.toEqual({ ...view, model: 'installed' });
  });
});
