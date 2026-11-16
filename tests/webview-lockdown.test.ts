/**
 * WebView lockdown inspection for #208 (ADR 0020): SEC-07a, SEC-07b (AT-17) and the CSP half of
 * SEC-20a (AT-37). Each check runs on the repo and on bad fixtures, so a bad config fails it.
 */
import { readFileSync, readdirSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  FETCH_DIRECTIVES,
  checkCspSources,
  checkForbiddenPlugins,
  checkHtmlEntries,
  checkWebviewSettings,
  checkWindowUrls,
  cspAllows,
  findFiles,
  type PluginInputs,
} from './webview-lockdown';

const root = resolve(import.meta.dirname, '..');
const read = (p: string): string => readFileSync(resolve(root, p), 'utf8');
const conf = JSON.parse(read('src-tauri/tauri.conf.json')) as {
  app: { security: { csp: string; devCsp: string } };
};
const csp = conf.app.security.csp;
const devCsp = conf.app.security.devCsp;
const capFiles = readdirSync(resolve(root, 'src-tauri/capabilities')).filter((f) => f.endsWith('.json'));
const caps = capFiles.map(
  (f) => JSON.parse(read(`src-tauri/capabilities/${f}`)) as PluginInputs['capabilities'][number],
);
const rustSources = [
  ...findFiles(root, 'src-tauri/src', (n) => n.endsWith('.rs')),
  ...findFiles(root, 'crates', (n) => n.endsWith('.rs')),
];
const cargoFiles = [
  { path: 'Cargo.lock', text: read('Cargo.lock') },
  { path: 'Cargo.toml', text: read('Cargo.toml') },
  ...findFiles(root, 'src-tauri', (n) => n === 'Cargo.toml'),
  ...findFiles(root, 'crates', (n) => n === 'Cargo.toml'),
];
const repoInputs = (): PluginInputs => ({
  capabilities: structuredClone(caps),
  tauriConf: structuredClone(conf),
  cargoFiles: [...cargoFiles],
  packageJson: JSON.parse(read('package.json')) as PluginInputs['packageJson'],
  rustSources: [...rustSources],
});

/** The app origins Tauri serves bundled files from: macOS and Linux, then Windows. */
const APP_ORIGINS = ['tauri://localhost', 'http://tauri.localhost'];
/** Hosts the Rive runtime and its files reach by default (wasm CDN, fallback CDN, asset CDN). */
const REMOTE_ASSET_URLS = [
  'https://unpkg.com/@rive-app/canvas@2.50.0/rive.wasm',
  'https://cdn.jsdelivr.net/npm/@rive-app/canvas@2.50.0/rive_fallback.wasm',
  'https://public.rive.app/cdn/uuid/abc',
  'https://example.com/mascot.riv',
  'http://example.com/mascot.riv',
  'wss://example.com/socket',
];

describe('[SEC-07a] [AT-17] each WebView CSP has no external source', () => {
  it('[SEC-07a] [AT-17] the release CSP allows only the app, the IPC transport and keywords', () => {
    expect(checkCspSources(csp)).toEqual([]);
  });

  it('[SEC-07a] [AT-17] the dev CSP adds only the loopback dev server', () => {
    expect(checkCspSources(devCsp, true)).toEqual([]);
  });

  it('[SEC-07a] [AT-17] no setting disables the CSP or gives a remote page IPC', () => {
    expect(checkWebviewSettings(conf, caps)).toEqual([]);
  });

  it('[SEC-07a] [AT-17] every window loads a bundled app page', () => {
    expect(checkWindowUrls(rustSources)).toEqual([]);
  });

  it('[SEC-07a] [AT-17] no HTML entry links an external file', () => {
    const html = ['brain.html', 'hub.html', 'mog.html', 'card.html'].map((p) => ({ path: p, text: read(p) }));
    expect(checkHtmlEntries(html)).toEqual([]);
  });

  it('[SEC-07a] [AT-17] self-test: the source check fails on hosts, wildcards and schemes', () => {
    const good = "default-src 'self'; connect-src 'self' ipc: http://ipc.localhost";
    expect(checkCspSources(good)).toEqual([]);
    for (const bad of [
      'https://api.example.com',
      'https:',
      'http:',
      '*',
      '*.databricks.com',
      'wss://x.example',
      'data:',
      'blob:',
      'ws://127.0.0.1:1420',
    ]) {
      expect(checkCspSources(`${good} ${bad}`), bad).toEqual([`connect-src allows ${bad}`]);
    }
    expect(checkCspSources(`${good}; img-src https://cdn.example`)).toEqual([
      'img-src allows https://cdn.example',
    ]);
    expect(checkCspSources("connect-src 'self'")).toEqual(['no default-src: unlisted fetches are open']);
    expect(checkCspSources("default-src 'self'")).toEqual(['connect-src must be explicit']);
    expect(checkCspSources(`${good} ws://127.0.0.1:1420`, true)).toEqual([]);
    expect(checkCspSources(`${good} ws://127.0.0.1:9999`, true)).toEqual([
      'connect-src allows ws://127.0.0.1:9999',
    ]);
  });

  it('[SEC-07a] [AT-17] self-test: the settings, window and HTML checks fail on bad fixtures', () => {
    const bad = structuredClone(conf) as unknown as {
      app: { security: Record<string, unknown>; windows: unknown[] };
    };
    bad.app.security['dangerousDisableAssetCspModification'] = true;
    bad.app.windows = [{ url: 'https://evil.example' }];
    expect(checkWebviewSettings(bad, [{ identifier: 'mog', remote: { urls: ['https://x'] } }])).toEqual([
      'dangerousDisableAssetCspModification is set',
      'window url https://evil.example is not an app URL',
      'capability mog has a remote scope',
    ]);
    expect(
      checkWindowUrls([{ path: 'w.rs', text: 'WebviewUrl::External("https://x".parse().unwrap())' }]),
    ).toEqual(['w.rs uses WebviewUrl::External']);
    expect(checkWindowUrls([{ path: 'w.rs', text: '// WebviewUrl::External is banned' }])).toEqual([]);
    expect(
      checkHtmlEntries([{ path: 'x.html', text: '<script src="https://cdn.example/a.js"></script>' }]),
    ).toEqual(['x.html links https://cdn.example/a.js']);
    expect(checkHtmlEntries([{ path: 'x.html', text: '<link href="//cdn.example/a.css">' }])).toEqual([
      'x.html links //cdn.example/a.css',
    ]);
  });
});

describe('[SEC-07a] [SEC-20a] [AT-37] the CSP blocks remote Rive hosts and allows bundled files', () => {
  it('[SEC-07a] [SEC-20a] [AT-37] no fetch directive can reach the Rive CDNs or any remote host', () => {
    for (const policy of [csp, devCsp]) {
      for (const origin of APP_ORIGINS) {
        for (const d of FETCH_DIRECTIVES) {
          for (const url of REMOTE_ASSET_URLS) {
            expect(cspAllows(policy, d, url, origin), `${d} ${url}`).toBe(false);
          }
        }
      }
    }
  });

  it('[SEC-20a] [AT-37] connect-src lets the runtime fetch the bundled rive.wasm and .riv', () => {
    for (const origin of APP_ORIGINS) {
      for (const file of ['/assets/rive.wasm', '/assets/mascot.riv']) {
        expect(cspAllows(csp, 'connect-src', `${origin}${file}`, origin), `${origin}${file}`).toBe(true);
      }
    }
  });

  it('[SEC-20a] [AT-37] script-src lets the bundled wasm compile', () => {
    expect(csp).toMatch(/script-src [^;]*'wasm-unsafe-eval'/);
  });

  it('[SEC-07a] [SEC-20a] self-test: the matcher allows what a loose policy allows', () => {
    const o = 'tauri://localhost';
    expect(cspAllows('default-src *', 'img-src', 'https://unpkg.com/x', o)).toBe(true);
    expect(cspAllows('connect-src https:', 'connect-src', 'https://unpkg.com/x', o)).toBe(true);
    expect(cspAllows('connect-src *.rive.app', 'connect-src', 'https://public.rive.app/cdn/x', o)).toBe(true);
    expect(cspAllows('connect-src http://unpkg.com', 'connect-src', 'https://unpkg.com/x', o)).toBe(true);
    expect(cspAllows("default-src 'self'", 'font-src', 'tauri://localhost/f.woff2', o)).toBe(true);
    expect(cspAllows("default-src 'none'", 'font-src', 'tauri://localhost/f.woff2', o)).toBe(false);
    expect(cspAllows("default-src 'self'", 'connect-src', 'https://unpkg.com/x', o)).toBe(false);
    expect(cspAllows("script-src 'self'", 'worker-src', 'https://unpkg.com/w.js', o)).toBe(false);
    expect(cspAllows('img-src data:', 'img-src', 'data:image/png;base64,AA', o)).toBe(true);
    expect(cspAllows('', 'img-src', 'https://unpkg.com/x', o)).toBe(true);
  });
});

describe('[SEC-07b] [AT-17] no WebView gets the http, shell or fs plugin', () => {
  it('[SEC-07b] [AT-17] capabilities, config, Cargo, npm and Rust sources name none of them', () => {
    expect(checkForbiddenPlugins(repoInputs())).toEqual([]);
  });

  it('[SEC-07b] [AT-17] the four per-window capability files are all inspected', () => {
    expect(capFiles.sort()).toEqual(['brain.json', 'card.json', 'hub.json', 'mog.json']);
  });

  it('[SEC-07b] [AT-17] self-test: each source of a forbidden plugin fails the check', () => {
    for (const plugin of ['http', 'shell', 'fs']) {
      const perm = repoInputs();
      perm.capabilities[0]?.permissions?.push(`${plugin}:default`);
      expect(checkForbiddenPlugins(perm)).toEqual([
        `capability ${perm.capabilities[0]?.identifier} grants ${plugin}:default`,
      ]);

      const obj = repoInputs();
      obj.capabilities[1]?.permissions?.push({ identifier: `${plugin}:allow-read`, allow: [] });
      expect(checkForbiddenPlugins(obj)).toHaveLength(1);

      const cfg = repoInputs();
      (cfg.tauriConf as Record<string, unknown>)['plugins'] = { [plugin]: {} };
      expect(checkForbiddenPlugins(cfg)).toEqual([`tauri.conf.json configures the ${plugin} plugin`]);

      const lock = repoInputs();
      lock.cargoFiles.push({ path: 'Cargo.lock', text: `[[package]]\nname = "tauri-plugin-${plugin}"\n` });
      expect(checkForbiddenPlugins(lock)).toEqual([`Cargo.lock names tauri-plugin-${plugin}`]);

      const toml = repoInputs();
      toml.cargoFiles.push({ path: 'src-tauri/Cargo.toml', text: `tauri-plugin-${plugin} = "2"\n` });
      expect(checkForbiddenPlugins(toml)).toEqual([`src-tauri/Cargo.toml names tauri-plugin-${plugin}`]);

      const npm = repoInputs();
      npm.packageJson.dependencies = { ...npm.packageJson.dependencies, [`@tauri-apps/plugin-${plugin}`]: '2.0.0' };
      expect(checkForbiddenPlugins(npm)).toEqual([`package.json depends on @tauri-apps/plugin-${plugin}`]);

      const rs = repoInputs();
      rs.rustSources.push({ path: 'src-tauri/src/lib.rs', text: `.plugin(tauri_plugin_${plugin}::init())` });
      expect(checkForbiddenPlugins(rs)).toEqual([`src-tauri/src/lib.rs uses tauri_plugin_${plugin}`]);
    }
  });

  it('[SEC-07b] self-test: comments and other plugins do not trip the check', () => {
    const ok = repoInputs();
    ok.cargoFiles.push({ path: 'a/Cargo.toml', text: '# no tauri-plugin-http, -shell or -fs\n' });
    ok.rustSources.push({ path: 'a.rs', text: '// never tauri_plugin_shell\n' });
    ok.capabilities[0]?.permissions?.push('core:event:allow-listen', 'allow-fs-like-command');
    expect(checkForbiddenPlugins(ok)).toEqual([]);
  });
});
