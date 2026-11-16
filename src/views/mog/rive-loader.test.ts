/**
 * SEC-20a (AT-37, 4.6 rule 4): the MOG view builds Rive in one place, with the asset CDN off,
 * the runtime wasm from the bundle, and the `.riv` file from a bundled URL only (ADR 0020).
 */
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import {
  RIVE_WASM_URL,
  configureOfflineRuntime,
  createOfflineRive,
  isBundledUrl,
  offlineRiveParams,
  type RiveDeps,
} from './rive-loader';

const root = resolve(import.meta.dirname, '../../..');

function fakeDeps() {
  const calls: { wasm: string[]; fallback: (string | null)[]; threads: (string | null)[] } = {
    wasm: [],
    fallback: [],
    threads: [],
  };
  const built: unknown[] = [];
  class FakeRive {
    constructor(params: unknown) {
      built.push(params);
    }
  }
  const deps = {
    Rive: FakeRive,
    loader: {
      setWasmUrl: (u: string) => calls.wasm.push(u),
      setWasmFallbackUrl: (u: string | null) => calls.fallback.push(u),
      setWasmThreadsUrl: (u: string | null) => calls.threads.push(u),
    },
  } as unknown as RiveDeps;
  return { deps, calls, built };
}

const canvas = {} as HTMLCanvasElement;

describe('[SEC-20a] [AT-37] offline Rive loader', () => {
  it('[SEC-20a] [AT-37] every Rive instance gets enableRiveAssetCDN: false', () => {
    const { deps, built } = fakeDeps();
    createOfflineRive({ src: '/assets/mascot.riv', canvas }, deps);
    expect(built).toHaveLength(1);
    expect(built[0]).toMatchObject({ src: '/assets/mascot.riv', canvas, enableRiveAssetCDN: false });
  });

  it('[SEC-20a] [AT-37] a caller cannot turn the asset CDN back on', () => {
    const sneaky = { src: '/assets/mascot.riv', canvas, enableRiveAssetCDN: true } as unknown as Parameters<
      typeof offlineRiveParams
    >[0];
    expect(offlineRiveParams(sneaky).enableRiveAssetCDN).toBe(false);
  });

  it('[SEC-20a] [AT-37] the runtime wasm comes from the bundle, with no CDN fallback', () => {
    const { deps, calls } = fakeDeps();
    configureOfflineRuntime(deps.loader);
    expect(calls.wasm).toEqual([RIVE_WASM_URL]);
    expect(calls.fallback).toEqual([null]);
    expect(calls.threads).toEqual([null]);
    expect(isBundledUrl(RIVE_WASM_URL)).toBe(true);
    expect(RIVE_WASM_URL).toMatch(/rive\.wasm$/);
  });

  it('[SEC-20a] [AT-37] createOfflineRive sets up the runtime before it builds Rive', () => {
    const { deps, calls } = fakeDeps();
    createOfflineRive({ src: '/assets/mascot.riv', canvas }, deps);
    expect(calls.wasm).toEqual([RIVE_WASM_URL]);
  });

  it('[SEC-20a] [AT-37] a remote .riv URL is refused before Rive is built', () => {
    for (const src of [
      'https://public.rive.app/cdn/uuid/abc',
      'http://example.com/m.riv',
      '//example.com/m.riv',
      'data:application/octet-stream;base64,AA',
      'blob:https://example.com/1',
      'HTTPS://EXAMPLE.COM/m.riv',
      '\\\\example.com\\m.riv',
      '',
    ]) {
      const { deps, built } = fakeDeps();
      expect(() => createOfflineRive({ src, canvas }, deps), src).toThrow(/bundled/);
      expect(built).toHaveLength(0);
    }
  });

  it('[SEC-20a] [AT-37] bundled paths are accepted', () => {
    for (const src of ['/assets/mascot-abc123.riv', './mascot.riv', 'assets/mascot.riv']) {
      expect(isBundledUrl(src), src).toBe(true);
    }
  });

  it('[SEC-20a] [AT-37] rive-loader.ts is the only app file that imports the Rive runtime', () => {
    const hits: string[] = [];
    const walk = (dir: string): void => {
      for (const e of readdirSync(dir, { withFileTypes: true })) {
        const p = join(dir, e.name);
        if (e.isDirectory()) walk(p);
        else if (/\.(ts|tsx|js|jsx)$/.test(e.name) && !/\.test\.tsx?$/.test(e.name)) {
          if (/from\s+['"]@rive-app\/|import\(\s*['"]@rive-app\//.test(readFileSync(p, 'utf8'))) {
            hits.push(relative(root, p));
          }
        }
      }
    };
    walk(join(root, 'src'));
    expect(hits).toEqual(['src/views/mog/rive-loader.ts']);
  });
});
