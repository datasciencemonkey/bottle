/**
 * Pin checks for the Tauri shell and the Rive runtime (#169, ADR 0012).
 * NFR-14a, NFR-14b (AT-22) and NFR-16a, NFR-16b (AT-37) are inspection rows: these pure functions
 * read the lockfiles and configs as text and return a list of problems. An empty list is a pass.
 * The tests in `packaging-pins.test.ts` run them on the repo and on bad fixtures.
 */
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, sep } from 'node:path';

/** NFR-14a, U1, D32: the lowest allowed version of the Rust `tauri` crate. */
export const MIN_TAURI_CRATE = '2.12.5';
/**
 * ADR 0012: the npm side of Tauri has no 2.12.5 release. The npm packages stay on the crate's
 * major.minor line, at this version or later.
 */
export const MIN_TAURI_NPM = '2.12.4';
export const TAURI_NPM_PACKAGES = ['@tauri-apps/api', '@tauri-apps/cli'] as const;
/** NFR-16a: the two runtimes that SP-01 can choose. */
export const ALLOWED_RIVE = ['@rive-app/webgl2', '@rive-app/canvas'] as const;
/** NFR-16b: the banned runtime. The pattern does not match `@rive-app/webgl2`. */
export const BANNED_RIVE = /@rive-app\/webgl(?![\w-])/;
/**
 * NFR-14b: `macOSPrivateApi` is allowed only when the SP-01 report records this fallback line
 * (SP-01 fallback step 3). ADR 0012 defines the line.
 */
export const SP01_PRIVATE_API_FALLBACK = '**SP-01 fallback recorded: macOSPrivateApi**';

const EXACT = /^\d+\.\d+\.\d+$/;

export interface PackageJson {
  dependencies?: Record<string, string>;
  devDependencies?: Record<string, string>;
  optionalDependencies?: Record<string, string>;
}

export interface TextFile {
  path: string;
  text: string;
}

export interface LockEntry {
  specifier: string;
  version: string;
}

/** Compare two `x.y.z` versions. Returns <0, 0 or >0. */
export function compareVersions(a: string, b: string): number {
  const pa = a.split('.').map(Number);
  const pb = b.split('.').map(Number);
  for (let i = 0; i < 3; i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d !== 0) return d;
  }
  return 0;
}

function majorMinor(v: string): string {
  return v.split('.').slice(0, 2).join('.');
}

/** The `[[package]]` entries of a Cargo.lock with the given name. */
export function cargoLockVersions(cargoLock: string, name: string): string[] {
  return cargoLock
    .split('[[package]]')
    .slice(1)
    .filter((block) => new RegExp(`^name = "${name}"$`, 'm').test(block))
    .map((block) => block.match(/^version = "([^"]+)"$/m)?.[1] ?? '');
}

/** [NFR-14a] One `tauri` crate in Cargo.lock, at MIN_TAURI_CRATE or later. */
export function checkTauriCrate(cargoLock: string): string[] {
  const versions = cargoLockVersions(cargoLock, 'tauri');
  if (versions.length === 0) return ['Cargo.lock has no tauri crate'];
  if (versions.length > 1)
    return [`Cargo.lock has more than one tauri crate: ${versions.join(', ')}`];
  const [v = ''] = versions;
  if (!EXACT.test(v)) return [`tauri crate version "${v}" is not x.y.z`];
  if (compareVersions(v, MIN_TAURI_CRATE) < 0) {
    return [`tauri crate ${v} is older than ${MIN_TAURI_CRATE} (NFR-14a)`];
  }
  return [];
}

/**
 * The direct dependencies of one importer in a pnpm v9 lockfile (dependencies, devDependencies
 * and optionalDependencies). The version keeps only the part before a peer suffix "(".
 */
export function pnpmImporter(lock: string, importer = '.'): Record<string, LockEntry> {
  const lines = lock.split(/\r?\n/);
  const start = lines.findIndex((l) => l === 'importers:');
  if (start < 0) throw new Error('pnpm lockfile has no importers section');
  const names = [importer, `'${importer}'`];
  let i = lines.findIndex((l, n) => n > start && names.some((m) => l === `  ${m}:`));
  if (i < 0) throw new Error(`pnpm lockfile has no importer "${importer}"`);
  const out: Record<string, LockEntry> = {};
  const depSections = new Set(['dependencies', 'devDependencies', 'optionalDependencies']);
  let inDeps = false;
  let current: string | undefined;
  for (i += 1; i < lines.length; i++) {
    const line = lines[i] ?? '';
    if (line.trim() === '') continue;
    const indent = line.length - line.trimStart().length;
    if (indent <= 2) break;
    if (indent === 4) {
      // A section such as dependencies or dependenciesMeta. Only the three dependency sections count.
      inDeps = depSections.has(line.trim().replace(/:$/, ''));
      current = undefined;
    } else if (!inDeps) {
      continue;
    } else if (indent === 6) {
      current = line
        .trim()
        .replace(/:$/, '')
        .replace(/^'(.*)'$/, '$1');
      out[current] = { specifier: '', version: '' };
    } else if (indent === 8 && current) {
      const m = line.trim().match(/^(specifier|version): (.*)$/);
      const entry = out[current];
      if (m && entry) {
        const value = (m[2] ?? '').replace(/^'(.*)'$/, '$1');
        if (m[1] === 'specifier') entry.specifier = value;
        else entry.version = value.split('(')[0] ?? '';
      }
    }
  }
  return out;
}

function allDeps(pkg: PackageJson): Record<string, string> {
  return { ...pkg.optionalDependencies, ...pkg.devDependencies, ...pkg.dependencies };
}

/**
 * [NFR-14a] (ADR 0012) Each npm Tauri package is an exact pin, on the same major.minor line as
 * the Rust crate, at MIN_TAURI_NPM or later, and the lockfile resolves the same version.
 */
export function checkTauriNpm(pkg: PackageJson, lock: string, crateVersion: string): string[] {
  const problems: string[] = [];
  const deps = allDeps(pkg);
  const locked = pnpmImporter(lock);
  for (const name of TAURI_NPM_PACKAGES) {
    const spec = deps[name];
    if (spec === undefined) {
      problems.push(`package.json has no ${name}`);
      continue;
    }
    if (!EXACT.test(spec)) {
      problems.push(`${name} "${spec}" is not an exact x.y.z pin`);
      continue;
    }
    if (majorMinor(spec) !== majorMinor(crateVersion)) {
      problems.push(`${name} ${spec} is not on the tauri crate line ${majorMinor(crateVersion)}`);
    }
    if (compareVersions(spec, MIN_TAURI_NPM) < 0) {
      problems.push(`${name} ${spec} is older than ${MIN_TAURI_NPM}`);
    }
    const entry = locked[name];
    if (!entry) problems.push(`pnpm-lock.yaml has no ${name}`);
    else if (entry.specifier !== spec || entry.version !== spec) {
      problems.push(
        `pnpm-lock.yaml has ${name} ${entry.specifier} -> ${entry.version}, not ${spec}`,
      );
    }
  }
  return problems;
}

/** The JSON paths of every key with this name, at any depth. */
export function findKeyDeep(value: unknown, key: string, path = '$'): string[] {
  if (Array.isArray(value)) {
    return value.flatMap((v, i) => findKeyDeep(v, key, `${path}[${i}]`));
  }
  if (value !== null && typeof value === 'object') {
    return Object.entries(value).flatMap(([k, v]) => [
      ...(k === key ? [`${path}.${k}`] : []),
      ...findKeyDeep(v, key, `${path}.${k}`),
    ]);
  }
  return [];
}

/**
 * [NFR-14b] No `macOSPrivateApi` key in any Tauri config file and no `macos-private-api` feature in
 * any Cargo.toml, unless the SP-01 report records the fallback line.
 */
export function checkNoPrivateApi(
  tauriConfigs: TextFile[],
  cargoTomls: TextFile[],
  sp01Report: string,
): string[] {
  if (sp01Report.split(/\r?\n/).some((l) => l.trim() === SP01_PRIVATE_API_FALLBACK)) return [];
  const problems: string[] = [];
  for (const f of tauriConfigs) {
    for (const p of findKeyDeep(JSON.parse(f.text) as unknown, 'macOSPrivateApi')) {
      problems.push(`${f.path} sets ${p} but SP-01 records no fallback`);
    }
  }
  for (const f of cargoTomls) {
    const code = f.text
      .split(/\r?\n/)
      .map((l) => l.replace(/#.*$/, ''))
      .join('\n');
    if (code.includes('macos-private-api')) {
      problems.push(
        `${f.path} enables the macos-private-api feature but SP-01 records no fallback`,
      );
    }
  }
  return problems;
}

/**
 * [NFR-16a] The app has exactly one Rive runtime, one of ALLOWED_RIVE, as an exact pin in
 * `dependencies`, and the lockfile resolves the same version. No other `@rive-app/*` direct dependency.
 */
export function checkRiveRuntime(pkg: PackageJson, lock: string): string[] {
  const problems: string[] = [];
  const direct = Object.keys(allDeps(pkg)).filter((d) => d.startsWith('@rive-app/'));
  const allowed: readonly string[] = ALLOWED_RIVE;
  const chosen = direct.filter((d) => allowed.includes(d));
  for (const d of direct.filter((x) => !allowed.includes(x))) {
    problems.push(`${d} is not an allowed Rive runtime (${ALLOWED_RIVE.join(' or ')})`);
  }
  if (chosen.length !== 1) {
    problems.push(
      `expected exactly one Rive runtime, found ${chosen.length}: ${chosen.join(', ')}`,
    );
    return problems;
  }
  const [name = ''] = chosen;
  const spec = pkg.dependencies?.[name];
  if (spec === undefined) {
    problems.push(`${name} must be in dependencies (it ships in the app)`);
    return problems;
  }
  if (!EXACT.test(spec)) problems.push(`${name} "${spec}" is not an exact x.y.z pin`);
  const entry = pnpmImporter(lock)[name];
  if (!entry) problems.push(`pnpm-lock.yaml has no ${name}`);
  else if (entry.specifier !== spec || entry.version !== spec) {
    problems.push(`pnpm-lock.yaml has ${name} ${entry.specifier} -> ${entry.version}, not ${spec}`);
  }
  if (!lock.includes(`\n  '${name}@${spec}':`)) {
    problems.push(`pnpm-lock.yaml packages section has no ${name}@${spec}`);
  }
  return problems;
}

/** [NFR-16b] No file names `@rive-app/webgl`. Returns one problem per file and line. */
export function checkNoWebgl(files: TextFile[]): string[] {
  return files.flatMap((f) =>
    f.text
      .split(/\r?\n/)
      .map((line, i) => (BANNED_RIVE.test(line) ? `${f.path}:${i + 1} names @rive-app/webgl` : ''))
      .filter((p) => p !== ''),
  );
}

const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'dist', 'reports', 'gen']);

/**
 * Every file under `root` whose name is in `names`. Skips build folders and never enters
 * `research/kimi-snapshot` (untrusted data).
 */
export function findFiles(root: string, names: (name: string) => boolean): TextFile[] {
  const out: TextFile[] = [];
  const walk = (dir: string): void => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, e.name);
      const rel = relative(root, full);
      if (e.isDirectory()) {
        if (SKIP_DIRS.has(e.name) || rel === join('research', 'kimi-snapshot')) continue;
        walk(full);
      } else if (e.isFile() && names(e.name)) {
        // POSIX separators, so the paths match on Windows too (#366).
        out.push({ path: rel.split(sep).join('/'), text: readFileSync(full, 'utf8') });
      }
    }
  };
  walk(root);
  return out;
}
