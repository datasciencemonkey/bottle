/**
 * UX-01b (inspection, AT-09): view windows never set the mood. The brain's MoodMapper (T11)
 * computes it; a view only renders the `mood` of the `mog.view_state` it gets.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const views = import.meta.dirname;

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? files(p) : /\.(ts|tsx)$/.test(f) ? [p] : [];
  });
}

const sources = files(views).filter(
  (f) => !/\.test\.tsx?$/.test(f) && !f.endsWith('test-tauri.ts'),
);
const MOOD = '(?:idle|listening|processing|answering|success|error|sleeping)';

/** The mood writes a view must not make. Each returns the offending text. */
export function moodWrites(text: string): string[] {
  const found: string[] = [];
  const add = (re: RegExp) => found.push(...[...text.matchAll(re)].map((m) => m[0]));
  // 1. No mood logic: the mapper, the agent tool or the priority list.
  add(/\b(?:createMoodMapper|mapMood|holdMood|MOOD_PRIORITY|set_mood)\b/g);
  // 2. No mood sent to the brain: a `view_emit` never carries a mood.
  add(/\bemitView\([^)]*\bmood\b[^)]*\)/g);
  add(/\bcall\(\s*['"`]view_emit['"`][^)]*\bmood\b[^)]*\)/g);
  // 3. The renderer gets only the brain's `<state>.mood`, never a literal or a computed value.
  add(/\bsetMood\((?!\s*[A-Za-z_$][\w$]*\??\.mood\s*\))(?!\s*mood\s*:)[^)]*\)/g);
  // 4. No literal mood stored outside the one UX-02b first frame (`mood=idle` at mount).
  add(new RegExp(`\\bmood['"\\]]*\\s*(?:=|:)\\s*['"\`]${MOOD}['"\`]`, 'g'));
  // 5. No mood set on the DOM by attribute, except from the brain's `<state>.mood`.
  add(/\bsetAttribute\(\s*['"`]data-mood['"`]\s*,(?!\s*[A-Za-z_$][\w$]*\??\.mood\s*\))[^)]*\)/g);
  return found;
}

/** UX-02b: the MOG view shows `idle` on the first frame, before the brain's first state. */
const FIRST_FRAME = { file: 'mog/mascot.ts', write: "mood'] = 'idle'" };

describe('UX-01b: views never set the mood', () => {
  it('[UX-01b] [AT-09] the scan flags each kind of mood write', () => {
    expect(moodWrites("import { createMoodMapper } from 'x';")).toHaveLength(1);
    expect(moodWrites("void emitView(id, 'click', { mood: 'idle' })")).toHaveLength(2);
    expect(moodWrites("mascot.setMood('error')")).toHaveLength(1);
    expect(moodWrites('mascot.setMood(pick(s))')).toHaveLength(1);
    expect(moodWrites("mascot.setMood( 'error')")).toHaveLength(1);
    expect(moodWrites("const s = { mood: 'success' }")).toHaveLength(1);
    expect(moodWrites("el.dataset['mood'] = 'sleeping'")).toHaveLength(1);
    expect(moodWrites("call('set_mood')")).toHaveLength(1);
    expect(moodWrites("void call('view_emit', { mogId, kind: 'click', mood })")).toHaveLength(1);
    expect(moodWrites("el.setAttribute('data-mood', 'error')")).toHaveLength(1);
  });

  it('[UX-01b] [AT-09] the scan lets a view render the brain state', () => {
    expect(moodWrites('mascot.current?.setMood(s.mood);')).toEqual([]);
    expect(moodWrites('mascot.current?.setMood( s.mood );')).toEqual([]);
    expect(moodWrites('setMood(mood: string): void {')).toEqual([]);
    expect(moodWrites("if (this.#el) this.#el.dataset['mood'] = mood;")).toEqual([]);
    expect(moodWrites("el.setAttribute('data-mood', s.mood)")).toEqual([]);
  });

  it('[UX-01b] [AT-09] no view source writes a mood', () => {
    expect(sources.length).toBeGreaterThan(5);
    for (const f of sources) {
      const rel = relative(views, f).replaceAll('\\', '/');
      const writes = moodWrites(readFileSync(f, 'utf8')).filter(
        (w) => !(rel === FIRST_FRAME.file && w.endsWith(FIRST_FRAME.write)),
      );
      expect(writes, rel).toEqual([]);
    }
  });

  it('[UX-01b] the only literal mood in a view is the first-frame idle (UX-02b)', () => {
    const text = readFileSync(join(views, FIRST_FRAME.file), 'utf8');
    expect(moodWrites(text)).toEqual([`mood${FIRST_FRAME.write.slice(4)}`]);
  });
});
