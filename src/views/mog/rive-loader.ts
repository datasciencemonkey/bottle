/**
 * The one place the MOG view builds Rive (SEC-20a, 4.6 rule 4, ADR 0021).
 *
 * - Every instance gets `enableRiveAssetCDN: false`, so Rive never fetches a referenced asset
 *   from its CDN.
 * - The runtime loads `rive.wasm` from the app bundle (Vite `?url`), never from unpkg, and the
 *   jsdelivr fallback wasm and the threads wasm are turned off.
 * - The `.riv` file must be a bundled path on the app origin; a remote URL throws.
 *
 * The CSP (`connect-src 'self' ipc: http://ipc.localhost`) blocks every remote host as a second
 * line, so a mistake here still loads nothing from the network.
 *
 * The mascot renderer (`RiveMascot`, ADR 0027) builds its Rive instance through
 * `createOfflineRive` with the bundled `.riv` URL. The mascot `.riv` embeds its assets; if it
 * ever uses referenced assets, add a bundled `assetLoader` here.
 */
import {
  DrawOptimizationOptions,
  Rive,
  RuntimeLoader,
  type RiveParameters,
} from '@rive-app/canvas';

/** Re-exported so this stays the only `src/` file that imports `@rive-app` (ADR 0021). */
export { DrawOptimizationOptions };
import riveWasmUrl from '@rive-app/canvas/rive.wasm?url';

/** The bundled runtime wasm (a path on the app origin). */
export const RIVE_WASM_URL: string = riveWasmUrl;

/** The RuntimeLoader calls this module makes. */
export interface RiveRuntimeLoader {
  setWasmUrl(url: string): void;
  setWasmFallbackUrl(url: string | null): void;
  setWasmThreadsUrl(url: string | null): void;
}

export interface RiveDeps {
  Rive: new (params: RiveParameters) => Rive;
  loader: RiveRuntimeLoader;
}

/** Options for an offline instance: everything but the source and the CDN flag. */
export type OfflineRiveOptions = Omit<
  RiveParameters,
  'src' | 'buffer' | 'riveFile' | 'enableRiveAssetCDN'
> & {
  /** A bundled path such as the Vite `?url` of the `.riv` file. */
  src: string;
};

/** `scheme://host[:port]`; it also works for `tauri:`, whose WHATWG `origin` is "null". */
function originOf(u: URL): string {
  return `${u.protocol}//${u.host}`;
}

/** The page the MOG view runs on, or the macOS app origin outside a browser (tests). */
function pageUrl(): string {
  return globalThis.location?.href ?? 'tauri://localhost/mog.html';
}

/**
 * A bundled URL is a path on the app origin: no scheme and no host. The URL parser strips
 * leading spaces and control characters and reads `\` as `/`, so those are refused first.
 * Then the path must resolve to the origin of `base` (the page URL by default).
 */
export function isBundledUrl(url: string, base: string = pageUrl()): boolean {
  if (url === '') return false;
  if (/[\s\\]/.test(url)) return false;
  if ([...url].some((c) => c.charCodeAt(0) < 0x20 || c.charCodeAt(0) === 0x7f)) return false;
  if (/^[a-z][a-z0-9+.-]*:/i.test(url) || url.startsWith('//')) return false;
  const page = new URL(base);
  return originOf(new URL(url, page)) === originOf(page);
}

/** Point the runtime at the bundled wasm and turn off its CDN fallbacks. */
export function configureOfflineRuntime(loader: RiveRuntimeLoader = RuntimeLoader): void {
  loader.setWasmUrl(RIVE_WASM_URL);
  loader.setWasmFallbackUrl(null);
  loader.setWasmThreadsUrl(null);
}

/** The Rive parameters for an offline instance. Throws on a remote `.riv` URL. */
export function offlineRiveParams(opts: OfflineRiveOptions): RiveParameters {
  if (!isBundledUrl(opts.src)) {
    throw new Error('The Rive file must be a bundled asset, not a remote URL');
  }
  // The flag comes last, so no option can turn the asset CDN back on.
  return { ...opts, enableRiveAssetCDN: false };
}

/** Build a Rive instance that loads only bundled files. */
export function createOfflineRive(
  opts: OfflineRiveOptions,
  deps: RiveDeps = { Rive, loader: RuntimeLoader },
): Rive {
  const params = offlineRiveParams(opts);
  configureOfflineRuntime(deps.loader);
  return new deps.Rive(params);
}
