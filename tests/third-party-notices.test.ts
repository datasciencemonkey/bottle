/**
 * Checks for THIRD_PARTY_NOTICES.md (#228). The expected notices come from section 20.1 of
 * `research/final-requirements.md`, so a change there makes this test fail until the file follows.
 */
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const NOTICES_PATH = resolve(root, 'THIRD_PARTY_NOTICES.md');
const requirements = readFileSync(resolve(root, 'research/final-requirements.md'), 'utf8');

/** The text of section 20.1, up to the next level-3 heading. */
function section201(md: string): string {
  const start = md.indexOf('### 20.1 Licence notices');
  if (start < 0) throw new Error('section 20.1 not found');
  const end = md.indexOf('\n### ', start + 1);
  return md.slice(start, end < 0 ? undefined : end);
}

interface Row {
  component: string;
  licence: string;
  notice: string;
}

function tableRows(md: string, header: string): Row[] {
  const lines = md.split('\n');
  const at = lines.findIndex((l) => l.startsWith(header));
  if (at < 0) throw new Error(`table "${header}" not found`);
  const rows: Row[] = [];
  for (const line of lines.slice(at + 2)) {
    if (!line.startsWith('|')) break;
    const cells = line.split('|').map((c) => c.trim());
    rows.push({
      component: cells[1] ?? '',
      licence: (cells[2] ?? '').replace(/\*\*/g, ''),
      notice: cells[3] ?? '',
    });
  }
  return rows;
}

/** "Tauri, `tauri-nspanel`" -> ["Tauri", "tauri-nspanel"]; "sherpa-onnx (k2-fsa)" -> ["sherpa-onnx"]. */
function componentNames(cell: string): string[] {
  return cell
    .replace(/\s*\(.*\)\s*$/, '')
    .split(', ')
    .map((n) => n.replace(/`/g, '').trim());
}

const s201 = section201(requirements);
const MIT_HEADER = (s201.match(/```\n([\s\S]*?)```/)?.[1] ?? '').trimEnd();
const REQ_ROWS = tableRows(s201, '| Component | Licence | Required notice |');
const PARAKEET = REQ_ROWS.find((r) => r.component.startsWith('NVIDIA Parakeet'))?.notice.match(
  /"([^"]+)"/,
)?.[1];
const HANDY_MIT = MIT_HEADER.replace(
  'Copyright (c) 2026 Sathish Gangichetty',
  'Copyright (c) 2025 CJ Pais',
);
const HANDY_HEADER =
  'Adapted from cjpais/Handy <path> @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.';
const PORTED_MARKER = 'Ported from antigravity-bricks-plugin (commit a31fd2c).';

/** The upstream copyright line of each MIT component (checked against the upstream LICENSE files). */
const MIT_COPYRIGHTS: ReadonlyArray<readonly [string, string]> = [
  ['antigravity-bricks-plugin', 'Copyright (c) 2026 Sathish Gangichetty'],
  ['Handy', 'Copyright (c) 2025 CJ Pais'],
  ['onnxruntime', 'Copyright (c) Microsoft Corporation'],
  ['Silero VAD v6', 'Copyright (c) 2020-present Silero Team'],
  ['Rive runtimes', 'Copyright (c) 2020-2021 Rive'],
  ['Tauri', 'Copyright (c) 2017 - Present Tauri Apps Contributors'],
  ['tauri-plugin-global-shortcut', 'Copyright (c) 2017 - Present Tauri Apps Contributors'],
  ['tauri-nspanel', 'Copyright (c) 2023 - Present Victor Aremu'],
];

function notices(): string {
  return readFileSync(NOTICES_PATH, 'utf8');
}

/** Split the file into `## heading` sections: heading text -> body. */
function sections(md: string): Map<string, string> {
  const out = new Map<string, string>();
  const parts = md.split(/^## /m).slice(1);
  for (const p of parts) {
    const nl = p.indexOf('\n');
    out.set(p.slice(0, nl).trim(), p.slice(nl + 1));
  }
  return out;
}

function textBlocks(body: string): string[] {
  return [...body.matchAll(/```text\n([\s\S]*?)```/g)].map((m) => (m[1] ?? '').trimEnd());
}

/** Remove one comment prefix (`//!`, `///`, `//`, `#`, ` *`) and one space from each line. */
function uncomment(text: string): string {
  return text
    .split('\n')
    .map((l) => l.replace(/^\s*(\/\/[!/]?|#|\/?\*+\/?)\s?/, '').trimEnd())
    .join('\n');
}

/** [SEC-21a] A file marked as ported must carry the full antigravity MIT header. */
function portedFileProblem(text: string, mitHeader: string): string | null {
  const plain = uncomment(text);
  if (!plain.split('\n').includes(PORTED_MARKER)) return null;
  return plain.includes(mitHeader) ? null : 'ported file lacks the antigravity MIT header';
}

const HANDY_LINE =
  /^(?:\/\/|#) Adapted from cjpais\/Handy (\S+) @ aca8985 \(MIT, Copyright \(c\) 2025 CJ Pais\)\. See THIRD_PARTY_NOTICES\.$/;

/** [SEC-21a] The Handy path named in a well-formed header, null if none, or an error string. */
function handyHeader(text: string): { path: string } | { error: string } | null {
  for (const line of text.split('\n')) {
    if (!/^\s*(?:\/\/[!/]?|#|\/?\*+)\s*Adapted from cjpais\/Handy\b/.test(line)) continue;
    const m = line.match(HANDY_LINE);
    return m?.[1] ? { path: m[1] } : { error: `malformed Handy header: ${line}` };
  }
  return null;
}

function sourceFiles(dir: string): string[] {
  if (!existsSync(dir)) return [];
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    if (name === 'node_modules' || name === 'target' || name.startsWith('.')) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...sourceFiles(p));
    else if (/\.(ts|tsx|js|mjs|rs|py)$/.test(name)) out.push(p);
  }
  return out;
}

const SCANNED = [
  'src',
  'src-tauri/src',
  'src-tauri/tests',
  'src-tauri/build.rs',
  'crates',
  'scripts',
  'tools',
  // Spike crates hold adapted Handy code too (SP-03, #5); `target/` is skipped above.
  'spikes',
];

function scannedFiles(): string[] {
  return SCANNED.flatMap((d) => {
    const p = resolve(root, d);
    if (!existsSync(p)) return [];
    return statSync(p).isDirectory() ? sourceFiles(p) : [p];
  });
}

/** Rows of the "Adapted files" table in the Handy section: Bottle path -> Handy path. */
function adaptedFiles(handyBody: string): Map<string, string> {
  const rows = tableRows(handyBody, '| Bottle file | Handy path |');
  return new Map(
    rows.map((r) => [r.component.replace(/`/g, ''), r.licence.replace(/`/g, '')] as const),
  );
}

describe('section 20.1 parse (guards the checks below)', () => {
  it('[SEC-21b] reads the MIT header, the component rows and the Parakeet sentence', () => {
    expect(MIT_HEADER.startsWith('MIT License\n\nCopyright (c) 2026 Sathish Gangichetty')).toBe(
      true,
    );
    expect(MIT_HEADER.endsWith('SOFTWARE.')).toBe(true);
    expect(REQ_ROWS.length).toBeGreaterThanOrEqual(8);
    expect(PARAKEET).toMatch(
      /^Speech recognition uses NVIDIA Parakeet TDT 0\.6B v3 .*int8 quantisation\.$/,
    );
    expect(REQ_ROWS.flatMap((r) => componentNames(r.component))).toEqual(
      expect.arrayContaining([
        'Tauri',
        'tauri-nspanel',
        'tauri-plugin-global-shortcut',
        'Rive runtimes',
      ]),
    );
  });
});

describe('THIRD_PARTY_NOTICES.md', () => {
  it('[AT-37] [SEC-21b] the file exists at the repository root', () => {
    expect(existsSync(NOTICES_PATH)).toBe(true);
  });

  it('[AT-37] [SEC-21b] the summary table has every row of section 20.1 with its licence', () => {
    const table = tableRows(notices(), '| Component | Licence | How Bottle uses it |');
    const got = new Map(table.map((r) => [r.component, r.licence]));
    for (const want of REQ_ROWS) expect(got.get(want.component), want.component).toBe(want.licence);
    expect(got.get('antigravity-bricks-plugin')).toBe('MIT');
    expect(got.get('Handy (cjpais/Handy)')).toBe('MIT');
  });

  it('[AT-37] [SEC-21b] each bundled component has its own section', () => {
    const headings = [...sections(notices()).keys()];
    const names = [
      'antigravity-bricks-plugin',
      'Handy',
      ...REQ_ROWS.flatMap((r) => componentNames(r.component)),
    ];
    for (const name of names) {
      expect(
        headings.some((h) => h.startsWith(`${name} (`)),
        `no "## ${name} (<licence>)" section`,
      ).toBe(true);
    }
  });

  it('[SEC-21b] the Parakeet CC-BY-4.0 attribution matches section 20.1 exactly', () => {
    expect(PARAKEET).toBeDefined();
    expect(notices()).toContain(PARAKEET);
  });

  it('[SEC-21a] [SEC-21b] the antigravity-bricks-plugin MIT header matches section 20.1', () => {
    const body = [...sections(notices())].find(([h]) =>
      h.startsWith('antigravity-bricks-plugin ('),
    );
    expect(body).toBeDefined();
    expect(textBlocks(body?.[1] ?? '')).toContain(MIT_HEADER);
  });

  it('[SEC-21b] every MIT section has a full MIT text and the file has the full Apache-2.0 text', () => {
    const md = notices();
    const permission = MIT_HEADER.slice(MIT_HEADER.indexOf('Permission is hereby granted'));
    for (const [heading, body] of sections(md)) {
      if (!/\(.*MIT.*\)$/.test(heading)) continue;
      const mit = textBlocks(body).find((b) => b.startsWith('MIT License\n\nCopyright '));
      expect(mit, `${heading} has no MIT text block`).toBeDefined();
      expect(mit?.endsWith(permission), `${heading} MIT text is not verbatim`).toBe(true);
    }
    for (const [component, copyright] of MIT_COPYRIGHTS) {
      const body = [...sections(md)].find(([h]) => h.startsWith(`${component} (`))?.[1] ?? '';
      expect(
        textBlocks(body).some((b) => b.startsWith(`MIT License\n\n${copyright}\n\n`)),
        `${component} lacks "${copyright}"`,
      ).toBe(true);
    }
    expect(md).toMatch(/Apache License\n\s+Version 2\.0, January 2004/);
    expect(md).toContain('END OF TERMS AND CONDITIONS');
  });

  it('[SEC-21a] the Handy entry names the commit and carries the full MIT text', () => {
    const entry = [...sections(notices())].find(([h]) => h === 'Handy (MIT)');
    expect(entry).toBeDefined();
    const body = entry?.[1] ?? '';
    expect(body).toContain('https://github.com/cjpais/Handy');
    expect(body).toContain('`aca8985`');
    expect(body).toContain(`// ${HANDY_HEADER}`);
    expect(textBlocks(body)).toContain(HANDY_MIT);
    expect(body).toContain('### Adapted files');
    expect(() => adaptedFiles(body)).not.toThrow();
  });

  it('[SEC-21a] a "How to add an entry" section comes before the summary table', () => {
    const md = notices();
    const howTo = md.indexOf('## How to add an entry');
    expect(howTo).toBeGreaterThan(0);
    expect(howTo).toBeLessThan(md.indexOf('| Component | Licence | How Bottle uses it |'));
  });
});

describe('ported and adapted file headers', () => {
  const header = (h: string, path = 'src-tauri/src/shortcut/mod.rs') =>
    `// ${h.replace('<path>', path)}`;

  it('[SEC-21a] the ported-file check accepts the full header and rejects a missing one', () => {
    const commented = MIT_HEADER.split('\n')
      .map((l) => (l ? `// ${l}` : '//'))
      .join('\n');
    expect(
      portedFileProblem(`// ${PORTED_MARKER}\n${commented}\nfn x() {}\n`, MIT_HEADER),
    ).toBeNull();
    expect(
      portedFileProblem(`// ${PORTED_MARKER}\n// MIT License\nfn x() {}\n`, MIT_HEADER),
    ).toMatch(/lacks/);
    expect(portedFileProblem('fn x() {}\n', MIT_HEADER)).toBeNull();
  });

  it('[SEC-21a] the Handy header check reads the path and rejects a malformed header', () => {
    expect(handyHeader(`${header(HANDY_HEADER)}\nfn x() {}\n`)).toEqual({
      path: 'src-tauri/src/shortcut/mod.rs',
    });
    expect(handyHeader('// Adapted from cjpais/Handy foo.rs\n')).toHaveProperty('error');
    expect(handyHeader(`//! ${HANDY_HEADER.replace('<path>', 'x.rs')}\n`)).toHaveProperty('error');
    expect(handyHeader(`  ${header(HANDY_HEADER)}\n`)).toHaveProperty('error');
    expect(handyHeader(` * ${HANDY_HEADER.replace('<path>', 'x.rs')}\n`)).toHaveProperty('error');
    expect(handyHeader('fn x() {}\n')).toBeNull();
  });

  it('[SEC-21a] every ported file keeps the antigravity MIT header', () => {
    const problems = scannedFiles()
      .map((f) => [relative(root, f), portedFileProblem(readFileSync(f, 'utf8'), MIT_HEADER)])
      .filter(([, p]) => p !== null);
    expect(problems).toEqual([]);
  });

  it('[SEC-21a] every Handy-adapted file has the header and a row in "Adapted files"', () => {
    const body = [...sections(notices())].find(([h]) => h === 'Handy (MIT)')?.[1] ?? '';
    const listed = adaptedFiles(body);
    const found = new Map<string, string>();
    const errors: string[] = [];
    for (const f of scannedFiles()) {
      const h = handyHeader(readFileSync(f, 'utf8'));
      if (!h) continue;
      if ('error' in h) errors.push(`${relative(root, f)}: ${h.error}`);
      else found.set(relative(root, f), h.path);
    }
    expect(errors).toEqual([]);
    expect(Object.fromEntries(found)).toEqual(Object.fromEntries(listed));
  });
});
