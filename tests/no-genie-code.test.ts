/**
 * ADR 0122: the old session host is gone. No brain, view, test-support or test file names it:
 * not in code, not in comments, not in UI text, not in fixtures. `src/shared` and the
 * contract-sync test (`tests/ipc-types.test.ts`, with its fixtures) go away with the Rust lane;
 * the files in `COMPAT` hold one token that the shared types still force on them.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
/** The old name, in any spelling: words, snake_case, kebab-case or camelCase. */
const OLD_NAME = /genie[ _-]?code/i;

/** Folders that this rule covers. */
const FOLDERS = ['src/brain', 'src/views', 'src/test-support', 'tests'];
/** Folders and files that the Rust lane owns. */
const SKIP = [
  'src/shared',
  'tests/fixtures',
  'tests/ipc-types.test.ts',
  'tests/no-genie-code.test.ts',
];

/**
 * The only tokens that stay, until the Rust lane drops them from the shared types. Each entry is
 * the exact text that may stay in that file. Delete an entry when its token goes.
 */
const COMPAT: Record<string, RegExp> = {
  // `HubAgentView` still requires the old status field.
  'src/brain/hub-agent-wiring.ts': /\bgenieCode\b/g,
  'src/brain/agent-failed-parity.test.ts': /\bgenieCode\b/g,
  'src/views/hub/ConversationsAgent.test.tsx': /\bgenieCode\b/g,
  // `COMMANDS` still lists the two old commands, and the cloud-call table needs a row for each.
  'src/test-support/cloud-calls.ts': /\bgenie_code_(status|install)\b/g,
};

function files(dir: string): string[] {
  return readdirSync(join(root, dir)).flatMap((name) => {
    const rel = `${dir}/${name}`;
    if (SKIP.some((s) => rel === s || rel.startsWith(`${s}/`))) return [];
    if (statSync(join(root, rel)).isDirectory()) return files(rel);
    return /\.(ts|tsx|css|json|md)$/.test(name) ? [rel] : [];
  });
}

describe('[ADR-0122] no mention of the old session host', () => {
  it('no brain, view, test-support or test file mentions it, comments and tests included', () => {
    const hits: string[] = [];
    for (const rel of FOLDERS.flatMap(files)) {
      let text = readFileSync(join(root, rel), 'utf8');
      const allowed = COMPAT[relative(root, join(root, rel))];
      if (allowed) text = text.replace(allowed, '');
      text.split('\n').forEach((line, i) => {
        if (OLD_NAME.test(line)) hits.push(`${rel}:${i + 1}`);
      });
    }
    expect(hits).toEqual([]);
  });

  it('every compat entry still has its token, so a stale entry is removed', () => {
    for (const [rel, re] of Object.entries(COMPAT)) {
      expect(readFileSync(join(root, rel), 'utf8').match(re), rel).not.toBeNull();
    }
  });
});
