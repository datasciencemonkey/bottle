/**
 * Lockfile and config inspection for #169 (ADR 0012). Each check runs on the repo and on bad
 * fixtures, so a bad lockfile or config provably fails it.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  ALLOWED_RIVE,
  MIN_TAURI_CRATE,
  SP01_PRIVATE_API_FALLBACK,
  cargoLockVersions,
  checkNoPrivateApi,
  checkNoWebgl,
  checkRiveRuntime,
  checkTauriCrate,
  checkTauriNpm,
  compareVersions,
  findFiles,
  pnpmImporter,
  type PackageJson,
} from './packaging-pins';

const root = resolve(import.meta.dirname, '..');
const read = (p: string): string => readFileSync(resolve(root, p), 'utf8');
const cargoLock = read('Cargo.lock');
const pkg = JSON.parse(read('package.json')) as PackageJson;
const lock = read('pnpm-lock.yaml');
const crate = cargoLockVersions(cargoLock, 'tauri')[0] ?? '';

// ---- fixtures ---------------------------------------------------------------------------------

const cargoBlock = (name: string, version: string): string =>
  `[[package]]\nname = "${name}"\nversion = "${version}"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n`;

/** A minimal pnpm v9 lockfile with the given root importer entries and package keys. */
function pnpmLock(
  deps: Record<string, [string, string]>,
  devDeps: Record<string, [string, string]> = {},
  packages: string[] = [],
): string {
  const section = (title: string, d: Record<string, [string, string]>): string =>
    Object.keys(d).length === 0
      ? ''
      : `    ${title}:\n` +
        Object.entries(d)
          .map(([n, [s, v]]) => `      '${n}':\n        specifier: ${s}\n        version: ${v}\n`)
          .join('');
  return [
    "lockfileVersion: '9.0'",
    '',
    'importers:',
    '',
    '  .:',
    section('dependencies', deps) + section('devDependencies', devDeps),
    'packages:',
    '',
    ...packages.map((p) => `  '${p}':\n    resolution: {integrity: sha512-x}\n`),
  ].join('\n');
}

const goodTauriPkg: PackageJson = {
  dependencies: { '@tauri-apps/api': '2.12.4' },
  devDependencies: { '@tauri-apps/cli': '2.12.4' },
};
const goodTauriLock = pnpmLock(
  { '@tauri-apps/api': ['2.12.4', '2.12.4'] },
  { '@tauri-apps/cli': ['2.12.4', '2.12.4'] },
);

const canvasPkg: PackageJson = { dependencies: { '@rive-app/canvas': '2.50.0' } };
const canvasLock = pnpmLock({ '@rive-app/canvas': ['2.50.0', '2.50.0'] }, {}, [
  '@rive-app/canvas@2.50.0',
]);

// ---- NFR-14a ----------------------------------------------------------------------------------

describe('[NFR-14a] [AT-22] Tauri shell pin', () => {
  it('[NFR-14a] [AT-22] Cargo.lock has one tauri crate at 2.12.5 or later', () => {
    expect(checkTauriCrate(cargoLock)).toEqual([]);
    expect(compareVersions(crate, MIN_TAURI_CRATE)).toBeGreaterThanOrEqual(0);
  });

  it('[NFR-14a] [AT-22] the workspace asks for tauri 2.12.5 or later', () => {
    const req = read('Cargo.toml').match(/^tauri = \{ version = "([^"]+)"/m)?.[1] ?? '';
    expect(req).toMatch(/^=?\d+\.\d+\.\d+$/);
    expect(compareVersions(req.replace(/^=/, ''), MIN_TAURI_CRATE)).toBeGreaterThanOrEqual(0);
  });

  it('[NFR-14a] [AT-22] self-test: the crate check fails on 2.12.4, on two tauri crates and on none', () => {
    expect(checkTauriCrate(cargoBlock('tauri', '2.12.4'))).toEqual([
      'tauri crate 2.12.4 is older than 2.12.5 (NFR-14a)',
    ]);
    expect(checkTauriCrate(cargoBlock('tauri', '2.11.9'))).toHaveLength(1);
    expect(
      checkTauriCrate(cargoBlock('tauri', '2.12.5') + cargoBlock('tauri', '2.10.0')),
    ).toHaveLength(1);
    expect(checkTauriCrate(cargoBlock('tauri-build', '2.12.5'))).toEqual([
      'Cargo.lock has no tauri crate',
    ]);
    expect(
      checkTauriCrate(cargoBlock('tauri', '2.12.5') + cargoBlock('tauri-utils', '2.10.2')),
    ).toEqual([]);
  });

  it('[NFR-14a] [AT-22] npm @tauri-apps/api and cli are exact pins on the crate line (ADR 0012)', () => {
    expect(checkTauriNpm(pkg, lock, crate)).toEqual([]);
  });

  it('[NFR-14a] [AT-22] self-test: the npm check fails on a range, an old line, an old patch and a lock mismatch', () => {
    expect(checkTauriNpm(goodTauriPkg, goodTauriLock, '2.12.5')).toEqual([]);
    const caret: PackageJson = {
      ...goodTauriPkg,
      dependencies: { '@tauri-apps/api': '^2.12.4' },
    };
    expect(checkTauriNpm(caret, goodTauriLock, '2.12.5')).toEqual([
      '@tauri-apps/api "^2.12.4" is not an exact x.y.z pin',
    ]);
    const oldLine: PackageJson = {
      dependencies: { '@tauri-apps/api': '2.11.9' },
      devDependencies: { '@tauri-apps/cli': '2.11.9' },
    };
    const oldLock = pnpmLock(
      { '@tauri-apps/api': ['2.11.9', '2.11.9'] },
      { '@tauri-apps/cli': ['2.11.9', '2.11.9'] },
    );
    expect(checkTauriNpm(oldLine, oldLock, '2.12.5').join('\n')).toMatch(
      /not on the tauri crate line/,
    );
    const oldPatch: PackageJson = {
      dependencies: { '@tauri-apps/api': '2.12.3' },
      devDependencies: { '@tauri-apps/cli': '2.12.4' },
    };
    const oldPatchLock = pnpmLock(
      { '@tauri-apps/api': ['2.12.3', '2.12.3'] },
      { '@tauri-apps/cli': ['2.12.4', '2.12.4'] },
    );
    expect(checkTauriNpm(oldPatch, oldPatchLock, '2.12.5')).toEqual([
      '@tauri-apps/api 2.12.3 is older than 2.12.4',
    ]);
    const drift = pnpmLock(
      { '@tauri-apps/api': ['2.12.4', '2.12.4'] },
      { '@tauri-apps/cli': ['2.12.4', '2.12.3'] },
    );
    expect(checkTauriNpm(goodTauriPkg, drift, '2.12.5')).toEqual([
      'pnpm-lock.yaml has @tauri-apps/cli 2.12.4 -> 2.12.3, not 2.12.4',
    ]);
    expect(checkTauriNpm({ dependencies: {} }, goodTauriLock, '2.12.5')).toHaveLength(2);
  });
});

// ---- NFR-14b ----------------------------------------------------------------------------------

describe('[NFR-14b] [AT-22] no macOSPrivateApi', () => {
  const tauriConfigs = findFiles(resolve(root, 'src-tauri'), (n) =>
    /^tauri(\.\w+)?\.conf\.json$/.test(n),
  );
  const cargoTomls = findFiles(root, (n) => n === 'Cargo.toml').filter(
    (f) => !f.path.startsWith('spikes'),
  );
  const sp01 = read('docs/spikes/SP-01.md');

  it('[NFR-14b] [AT-22] tauri.conf has no macOSPrivateApi and no Cargo.toml enables macos-private-api', () => {
    expect(tauriConfigs.map((f) => f.path)).toContain('tauri.conf.json');
    expect(cargoTomls.map((f) => f.path)).toContain('src-tauri/Cargo.toml');
    expect(checkNoPrivateApi(tauriConfigs, cargoTomls, sp01)).toEqual([]);
  });

  it('[NFR-14b] [AT-22] self-test: the check fails on the flag at any depth and on the cargo feature', () => {
    const flat = { path: 'a.conf.json', text: '{"app":{"macOSPrivateApi":true}}' };
    const nested = { path: 'b.conf.json', text: '{"app":{"x":[{"macOSPrivateApi":false}]}}' };
    const feature = {
      path: 'Cargo.toml',
      text: 'tauri = { version = "2.12.5", features = ["macos-private-api"] }\n',
    };
    const comment = { path: 'c/Cargo.toml', text: '# no macos-private-api here (NFR-14b)\n' };
    expect(checkNoPrivateApi([flat, nested], [feature, comment], '')).toEqual([
      'a.conf.json sets $.app.macOSPrivateApi but SP-01 records no fallback',
      'b.conf.json sets $.app.x[0].macOSPrivateApi but SP-01 records no fallback',
      'Cargo.toml enables the macos-private-api feature but SP-01 records no fallback',
    ]);
  });

  it('[NFR-14b] [AT-22] self-test: the flag is allowed only when SP-01 records the fallback line', () => {
    const flat = { path: 'a.conf.json', text: '{"app":{"macOSPrivateApi":true}}' };
    expect(checkNoPrivateApi([flat], [], `# SP-01\n\n${SP01_PRIVATE_API_FALLBACK}\n`)).toEqual([]);
    expect(
      checkNoPrivateApi([flat], [], `mentions ${SP01_PRIVATE_API_FALLBACK} inline`),
    ).toHaveLength(1);
    expect(sp01.split('\n').map((l) => l.trim())).not.toContain(SP01_PRIVATE_API_FALLBACK);
  });
});

// ---- NFR-16a / NFR-16b ------------------------------------------------------------------------

describe('[NFR-16a] [NFR-16b] [AT-37] Rive runtime pin', () => {
  it('[NFR-16a] [AT-37] package.json and the lockfile have exactly one of webgl2 or canvas, exact-pinned', () => {
    expect(checkRiveRuntime(pkg, lock)).toEqual([]);
  });

  it('[NFR-16a] [AT-37] the runtime is the SP-01 provisional choice @rive-app/canvas (TODO(#10))', () => {
    // TODO(#10): SP-01 section 7 chose canvas provisionally; the signed rerun confirms or changes it.
    const chosen = Object.keys(pkg.dependencies ?? {}).filter((d) => d.startsWith('@rive-app/'));
    expect(chosen).toEqual(['@rive-app/canvas']);
    expect(pnpmImporter(lock)['@rive-app/canvas']?.version).toBe(
      pkg.dependencies?.['@rive-app/canvas'],
    );
  });

  it('[NFR-16a] [AT-37] self-test: the runtime check fails on none, both, a range, devDependencies and a lock mismatch', () => {
    expect(checkRiveRuntime(canvasPkg, canvasLock)).toEqual([]);
    expect(checkRiveRuntime({ dependencies: {} }, canvasLock)).toEqual([
      'expected exactly one Rive runtime, found 0: ',
    ]);
    const both: PackageJson = {
      dependencies: { '@rive-app/canvas': '2.50.0', '@rive-app/webgl2': '2.50.0' },
    };
    expect(checkRiveRuntime(both, canvasLock)).toHaveLength(1);
    const range: PackageJson = { dependencies: { '@rive-app/canvas': '^2.50.0' } };
    expect(checkRiveRuntime(range, canvasLock).join('\n')).toMatch(/not an exact x\.y\.z pin/);
    const dev: PackageJson = { devDependencies: { '@rive-app/canvas': '2.50.0' } };
    expect(checkRiveRuntime(dev, canvasLock)).toEqual([
      '@rive-app/canvas must be in dependencies (it ships in the app)',
    ]);
    const drift = pnpmLock({ '@rive-app/canvas': ['2.50.0', '2.49.0'] }, {}, [
      '@rive-app/canvas@2.49.0',
    ]);
    expect(checkRiveRuntime(canvasPkg, drift)).toEqual([
      'pnpm-lock.yaml has @rive-app/canvas 2.50.0 -> 2.49.0, not 2.50.0',
      'pnpm-lock.yaml packages section has no @rive-app/canvas@2.50.0',
    ]);
    const other: PackageJson = {
      dependencies: { '@rive-app/canvas': '2.50.0', '@rive-app/webgl-advanced': '2.50.0' },
    };
    expect(checkRiveRuntime(other, canvasLock)).toEqual([
      '@rive-app/webgl-advanced is not an allowed Rive runtime (@rive-app/webgl2 or @rive-app/canvas)',
    ]);
  });

  it('[NFR-16a] [NFR-14a] [AT-37] self-test: the lockfile parser reads CRLF files and skips dependenciesMeta', () => {
    const crlf = (t: string): string => t.replace(/\n/g, '\r\n');
    expect(checkRiveRuntime(pkg, crlf(lock))).toEqual([]);
    expect(checkTauriNpm(pkg, crlf(lock), crate)).toEqual([]);
    expect(checkRiveRuntime(canvasPkg, crlf(canvasLock))).toEqual([]);
    expect(checkNoWebgl([{ path: 'x.ts', text: crlf("a\nimport '@rive-app/webgl';\n") }])).toEqual([
      'x.ts:2 names @rive-app/webgl',
    ]);
    const meta = canvasLock.replace(
      'packages:',
      "    dependenciesMeta:\n      '@rive-app/canvas':\n        injected: true\n\npackages:",
    );
    expect(meta).toContain('dependenciesMeta:');
    expect(pnpmImporter(meta)['@rive-app/canvas']).toEqual({
      specifier: '2.50.0',
      version: '2.50.0',
    });
    expect(checkRiveRuntime(canvasPkg, meta)).toEqual([]);
  });

  it('[NFR-16b] [AT-37] lockfile has no @rive-app/webgl', () => {
    const files = findFiles(root, (n) =>
      ['package.json', 'pnpm-lock.yaml', 'package-lock.json', 'yarn.lock'].includes(n),
    );
    const paths = files.map((f) => f.path);
    expect(paths).toContain('pnpm-lock.yaml');
    expect(paths).toContain('package.json');
    expect(checkNoWebgl(files)).toEqual([]);
  });

  it('[NFR-16b] [AT-37] no app source imports @rive-app/webgl', () => {
    const src = findFiles(resolve(root, 'src'), (n) => /\.(ts|tsx|js|mjs)$/.test(n));
    expect(src.length).toBeGreaterThan(0);
    expect(checkNoWebgl(src)).toEqual([]);
  });

  it('[NFR-16b] [AT-37] self-test: the ban fails on a bad lockfile and ignores webgl2', () => {
    const bad = pnpmLock({ '@rive-app/webgl': ['2.50.0', '2.50.0'] }, {}, [
      '@rive-app/webgl@2.50.0',
    ]);
    expect(checkNoWebgl([{ path: 'pnpm-lock.yaml', text: bad }])).toEqual([
      'pnpm-lock.yaml:7 names @rive-app/webgl',
      'pnpm-lock.yaml:13 names @rive-app/webgl',
    ]);
    expect(
      checkNoWebgl([{ path: 'x.ts', text: "import { Rive } from '@rive-app/webgl';" }]),
    ).toHaveLength(1);
    const ok = pnpmLock({ '@rive-app/webgl2': ['2.50.0', '2.50.0'] }, {}, [
      '@rive-app/webgl2@2.50.0',
    ]);
    expect(checkNoWebgl([{ path: 'pnpm-lock.yaml', text: ok }])).toEqual([]);
    expect(ALLOWED_RIVE).toEqual(['@rive-app/webgl2', '@rive-app/canvas']);
  });

  it('[NFR-16a] [SEC-21a] THIRD_PARTY_NOTICES names the chosen Rive runtime and its version', () => {
    const notices = read('THIRD_PARTY_NOTICES.md');
    const start = notices.indexOf('## Rive runtimes (MIT)');
    expect(start).toBeGreaterThanOrEqual(0);
    const section = notices.slice(start, notices.indexOf('\n## ', start + 1));
    const allowed: readonly string[] = ALLOWED_RIVE;
    const chosen = Object.keys(pkg.dependencies ?? {}).filter((d) => allowed.includes(d));
    expect(chosen).toHaveLength(1);
    const [name = 'missing'] = chosen;
    const version = pkg.dependencies?.[name] ?? 'missing';
    expect(section).toContain(`\`${name}\` ${version}`);
  });
});
