/**
 * UX-09a / UX-09b / SEC-20a (ADR 0027): the build ships the mascot `.riv` as a bundled file when
 * `rive mog-rive --publish` has built it, and ships without it otherwise (the view then shows the
 * PNG fallback). No `.riv` file is ever tracked in git.
 */
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import config, { MOG_RIV_ASSET, MOG_RIV_BUILD, mogRivAsset } from '../vite.config';

const root = resolve(import.meta.dirname, '..');

type Emitted = { type: string; fileName: string; source: Uint8Array };

function runGenerate(file: string) {
  const emitted: Emitted[] = [];
  const warnings: string[] = [];
  const plugin = mogRivAsset(file);
  const ctx = { emitFile: (e: Emitted) => emitted.push(e), warn: (w: string) => warnings.push(w) };
  (plugin.generateBundle as unknown as (this: typeof ctx) => void).call(ctx);
  return { emitted, warnings };
}

describe('the bundled mascot .riv (ADR 0027)', () => {
  it('[UX-09a] [SEC-20a] emits the built .riv at mascot/mog.riv on the app origin', () => {
    const dir = mkdtempSync(join(tmpdir(), 'mog-riv-'));
    const file = join(dir, 'mog-rive.riv');
    writeFileSync(file, Uint8Array.from([82, 73, 86, 69]));
    const { emitted, warnings } = runGenerate(file);
    expect(MOG_RIV_ASSET).toBe('mascot/mog.riv');
    expect(emitted).toHaveLength(1);
    expect(emitted[0]?.type).toBe('asset');
    expect(emitted[0]?.fileName).toBe('mascot/mog.riv');
    expect(Array.from(emitted[0]?.source ?? [])).toEqual([82, 73, 86, 69]);
    expect(warnings).toEqual([]);
  });

  it('[UX-09b] builds without the .riv and warns that the PNG fallback will show', () => {
    const { emitted, warnings } = runGenerate(join(tmpdir(), 'no-such-dir-mog', 'mog-rive.riv'));
    expect(emitted).toEqual([]);
    expect(warnings).toHaveLength(1);
    expect(warnings[0]).toMatch(/PNG fallback/);
  });

  it('[UX-09b] the default source is the gitignored rive CLI output', () => {
    expect(MOG_RIV_BUILD).toBe(join(root, 'mog-rive', 'build', 'mog-rive.riv'));
    const ignored = execFileSync('git', ['check-ignore', '-q', '--no-index', MOG_RIV_BUILD], {
      cwd: root,
    });
    expect(ignored.toString()).toBe('');
  });

  it('[UX-09b] [SEC-07a] no asset is inlined as a data: URL, which the CSP blocks', () => {
    expect(config.build?.assetsInlineLimit).toBe(0);
    expect(config.plugins?.some((p) => (p as { name?: string })?.name === 'bottle-mog-riv')).toBe(
      true,
    );
  });

  it('[UX-09b] no .riv file is tracked in git', () => {
    const tracked = execFileSync('git', ['ls-files', '*.riv', '**/*.riv'], { cwd: root });
    expect(tracked.toString()).toBe('');
  });
});
