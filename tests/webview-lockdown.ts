/**
 * WebView lockdown inspection for #208 (ADR 0020).
 * SEC-07a and SEC-07b are inspection rows (AT-17): these pure functions read the CSP, the
 * capability files, the Cargo and npm manifests and the Rust sources as text, and return a list
 * of problems. An empty list is a pass. `webview-lockdown.test.ts` runs them on the repo and on
 * bad fixtures, so a bad config provably fails them.
 */
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';

export interface TextFile {
  path: string;
  text: string;
}

/** One CSP as `directive -> sources`. Directive names are lower case. */
export type Csp = Map<string, string[]>;

export function parseCsp(csp: string): Csp {
  const out: Csp = new Map();
  for (const part of csp.split(';')) {
    const [name, ...sources] = part.trim().split(/\s+/);
    if (!name) continue;
    const key = name.toLowerCase();
    // CSP rule: the first copy of a directive wins.
    if (!out.has(key)) out.set(key, sources);
  }
  return out;
}

/** The fetch directives that can load a resource from a host. */
export const FETCH_DIRECTIVES = [
  'connect-src',
  'script-src',
  'style-src',
  'img-src',
  'font-src',
  'media-src',
  'object-src',
  'frame-src',
  'worker-src',
  'child-src',
  'manifest-src',
] as const;

const FALLBACK: Record<string, string[]> = {
  'worker-src': ['child-src', 'script-src', 'default-src'],
  'frame-src': ['child-src', 'default-src'],
};

/** The sources that apply to a fetch directive, with the CSP Level 3 fallback chain. */
export function effectiveSources(csp: Csp, directive: string): string[] {
  const chain = [directive, ...(FALLBACK[directive] ?? ['default-src'])];
  for (const d of chain) {
    const s = csp.get(d);
    if (s) return s;
  }
  return ['*'];
}

/** Source keywords and hashes that do not name a host. */
const KEYWORD = /^'(self|none|wasm-unsafe-eval|strict-dynamic|(nonce|sha256|sha384|sha512)-[^']+)'$/;
/** The Tauri IPC transports (macOS and Linux use `ipc:`, Windows uses `http://ipc.localhost`). */
const IPC_SOURCES = new Set(['ipc:', 'http://ipc.localhost']);
/** The Vite dev server on loopback (`devCsp` only, never the release CSP). */
const DEV_LOOPBACK = /^(https?|wss?):\/\/(127\.0\.0\.1|localhost):1420$/;

/**
 * SEC-07a: every source of every directive must be the app itself, the Tauri IPC transport, or
 * a keyword or hash. `dev` also allows the loopback dev server. Anything else (a host, `*`,
 * `http:`, `https:`, `ws:`, `wss:`, `data:`, `blob:`) is a problem.
 */
export function checkCspSources(csp: string, dev = false): string[] {
  const problems: string[] = [];
  const parsed = parseCsp(csp);
  if (!parsed.has('default-src')) problems.push('no default-src: unlisted fetches are open');
  if (!parsed.has('connect-src')) problems.push('connect-src must be explicit');
  for (const [name, sources] of parsed) {
    for (const s of sources) {
      if (KEYWORD.test(s) || IPC_SOURCES.has(s)) continue;
      if (dev && DEV_LOOPBACK.test(s)) continue;
      problems.push(`${name} allows ${s}`);
    }
  }
  return problems;
}

/** The origin of a URL as `scheme://host[:port]`, also for custom schemes such as `tauri:`. */
function originOf(u: URL): string {
  return `${u.protocol}//${u.host}`;
}

const NETWORK_SCHEMES = new Set(['http:', 'https:', 'ws:', 'wss:']);

function matchHostSource(source: string, url: URL, self: URL): boolean {
  const m = /^(?:([a-z][a-z0-9+.-]*):\/\/)?(\*\.)?([^/:]+|\*)(?::(\d+|\*))?(\/.*)?$/i.exec(source);
  if (!m) return false;
  const [, scheme, wildcard, host = '', port, path] = m;
  if (scheme) {
    const want = `${scheme.toLowerCase()}:`;
    const upgrade =
      (want === 'http:' && url.protocol === 'https:') || (want === 'ws:' && url.protocol === 'wss:');
    if (url.protocol !== want && !upgrade) return false;
  } else if (url.protocol !== self.protocol && !NETWORK_SCHEMES.has(url.protocol)) {
    return false;
  }
  const h = url.hostname.toLowerCase();
  const sh = host.toLowerCase();
  if (sh !== '*') {
    if (wildcard ? !h.endsWith(`.${sh}`) : h !== sh) return false;
  }
  if (port && port !== '*' && url.port !== port) return false;
  if (path && path !== '/' && !url.pathname.startsWith(path)) return false;
  return true;
}

/**
 * A small CSP source matcher: does `csp` let `directive` load `url` on a page at `selfOrigin`?
 * It covers 'self', 'none', `*`, scheme sources and host sources, which is all this app uses.
 */
export function cspAllows(csp: string, directive: string, url: string, selfOrigin: string): boolean {
  const target = new URL(url);
  const self = new URL(selfOrigin);
  const sources = effectiveSources(parseCsp(csp), directive);
  return sources.some((s) => {
    if (s === "'none'") return false;
    if (s === "'self'") return originOf(target) === originOf(self);
    if (s.startsWith("'")) return false;
    if (s === '*') return NETWORK_SCHEMES.has(target.protocol);
    if (/^[a-z][a-z0-9+.-]*:$/i.test(s)) return target.protocol === s.toLowerCase();
    return matchHostSource(s, target, self);
  });
}

/**
 * SEC-07a: no Tauri setting turns the CSP off or opens IPC to a remote page.
 * `conf` is the parsed `tauri.conf.json`; `caps` are the parsed capability files.
 */
export function checkWebviewSettings(conf: unknown, caps: unknown[]): string[] {
  const problems: string[] = [];
  const app = (conf as { app?: Record<string, unknown> }).app ?? {};
  const security = (app['security'] ?? {}) as Record<string, unknown>;
  if (typeof security['csp'] !== 'string') problems.push('app.security.csp must be a string');
  if (security['dangerousDisableAssetCspModification'] !== undefined) {
    problems.push('dangerousDisableAssetCspModification is set');
  }
  if (security['dangerousRemoteDomainIpcAccess'] !== undefined) {
    problems.push('dangerousRemoteDomainIpcAccess is set');
  }
  for (const w of (app['windows'] ?? []) as { url?: string }[]) {
    if (w.url && /^[a-z][a-z0-9+.-]*:/i.test(w.url)) problems.push(`window url ${w.url} is not an app URL`);
  }
  for (const c of caps as { identifier?: string; remote?: unknown }[]) {
    if (c.remote !== undefined) problems.push(`capability ${c.identifier ?? '?'} has a remote scope`);
  }
  return problems;
}

/** SEC-07a: every window loads a bundled page (`WebviewUrl::App`), never a remote one. */
export function checkWindowUrls(rustSources: TextFile[]): string[] {
  const problems: string[] = [];
  for (const f of rustSources) {
    const code = stripRustComments(f.text);
    for (const kind of ['WebviewUrl::External', 'WebviewUrl::CustomProtocol']) {
      if (code.includes(kind)) problems.push(`${f.path} uses ${kind}`);
    }
    if (/\.navigate\s*\(/.test(code)) problems.push(`${f.path} navigates a WebView`);
  }
  return problems;
}

/** SEC-07a: no HTML entry links a script, style or image by scheme or protocol-relative URL. */
export function checkHtmlEntries(html: TextFile[]): string[] {
  const problems: string[] = [];
  for (const f of html) {
    for (const m of f.text.matchAll(/\b(src|href)\s*=\s*["']([^"']*)["']/gi)) {
      const url = m[2] ?? '';
      if (/^[a-z][a-z0-9+.-]*:/i.test(url) || url.startsWith('//')) {
        problems.push(`${f.path} links ${url}`);
      }
    }
  }
  return problems;
}

/** SEC-07b: the three plugins no WebView may get. */
export const FORBIDDEN_PLUGINS = ['http', 'shell', 'fs'] as const;
const PLUGIN = FORBIDDEN_PLUGINS.join('|');

function stripRustComments(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
}

function stripTomlComments(text: string): string {
  return text.replace(/#.*$/gm, '');
}

export interface PluginInputs {
  /** Parsed capability files. */
  capabilities: { identifier?: string; permissions?: unknown[] }[];
  /** Parsed `tauri.conf.json`. */
  tauriConf: unknown;
  /** `Cargo.lock` and every workspace `Cargo.toml`. */
  cargoFiles: TextFile[];
  /** Parsed `package.json`. */
  packageJson: {
    dependencies?: Record<string, string>;
    devDependencies?: Record<string, string>;
    optionalDependencies?: Record<string, string>;
  };
  /** Every Rust source of the app and its crates. */
  rustSources: TextFile[];
}

/**
 * SEC-07b: no capability grants an `http:`, `shell:` or `fs:` permission, no config enables
 * those plugins, and no manifest or source pulls in `tauri-plugin-http/-shell/-fs` or
 * `@tauri-apps/plugin-http/-shell/-fs`.
 */
export function checkForbiddenPlugins(input: PluginInputs): string[] {
  const problems: string[] = [];
  const perm = new RegExp(`^(${PLUGIN}):`);
  for (const cap of input.capabilities) {
    for (const p of cap.permissions ?? []) {
      const id = typeof p === 'string' ? p : (p as { identifier?: string }).identifier ?? '';
      if (perm.test(id)) problems.push(`capability ${cap.identifier ?? '?'} grants ${id}`);
    }
  }
  const plugins = ((input.tauriConf as { plugins?: Record<string, unknown> }).plugins ?? {}) as Record<
    string,
    unknown
  >;
  for (const name of FORBIDDEN_PLUGINS) {
    if (name in plugins) problems.push(`tauri.conf.json configures the ${name} plugin`);
  }
  const crate = new RegExp(`\\btauri-plugin-(${PLUGIN})\\b`);
  for (const f of input.cargoFiles) {
    const m = crate.exec(stripTomlComments(f.text));
    if (m) problems.push(`${f.path} names ${m[0]}`);
  }
  const pkg = input.packageJson;
  const npm = new RegExp(`^@tauri-apps/plugin-(${PLUGIN})$`);
  for (const d of Object.keys({ ...pkg.dependencies, ...pkg.devDependencies, ...pkg.optionalDependencies })) {
    if (npm.test(d)) problems.push(`package.json depends on ${d}`);
  }
  const rust = new RegExp(`\\btauri_plugin_(${PLUGIN})\\b`);
  for (const f of input.rustSources) {
    const m = rust.exec(stripRustComments(f.text));
    if (m) problems.push(`${f.path} uses ${m[0]}`);
  }
  return problems;
}

/** Every file under `dir` (relative to `root`) whose name passes `keep`, skipping build output. */
export function findFiles(root: string, dir: string, keep: (name: string) => boolean): TextFile[] {
  const out: TextFile[] = [];
  const walk = (d: string): void => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      if (e.name === 'node_modules' || e.name === 'target' || e.name === 'gen') continue;
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (keep(e.name)) out.push({ path: relative(root, p), text: readFileSync(p, 'utf8') });
    }
  };
  walk(join(root, dir));
  return out;
}
