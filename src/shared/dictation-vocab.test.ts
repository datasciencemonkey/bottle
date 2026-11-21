import { describe, expect, it } from 'vitest';
import {
  clearUserWords,
  correctDictation,
  createDictationCorrector,
  DICTATION_WORDS_STORAGE_KEY,
  parseUserWords,
  readUserWordsText,
  writeUserWordsText,
} from './dictation-vocab';

describe('built-in corrections', () => {
  it.each([
    ['Can I create a G Way N API?', 'Can I create a Gateway API?'],
    ['can i create a gee way n api', 'can i create a Gateway api'],
    ['Use uni tea catalog for that.', 'Use Unity Catalog for that.'],
    ['Open lake base, please.', 'Open Lakebase, please.'],
    ['I work at Data Bricks.', 'I work at Databricks.'],
    ['Log it with ML flow today', 'Log it with MLflow today'],
    ['Ask the jeannie space about sales', 'Ask the Genie space about sales'],
    ['G WAY N, then more', 'Gateway, then more'],
  ])('%s', (input, out) => {
    expect(correctDictation(input)).toBe(out);
  });

  it('keeps the punctuation and the text around a match', () => {
    expect(correctDictation('(lake base) "ml flow"; data bricks!')).toBe(
      '(Lakebase) "MLflow"; Databricks!',
    );
  });

  it('does not match inside a longer word', () => {
    expect(correctDictation('The big way n ahead')).toBe('The big way n ahead');
    expect(correctDictation('regway nothing, lake bases, metadata bricks')).toBe(
      'regway nothing, lake bases, metadata bricks',
    );
    expect(correctDictation('my g wayne')).toBe('my g wayne');
  });

  it('never changes words that are not on a list', () => {
    const s = 'Open the gateway, call Genie and the data team about delta tables.';
    expect(correctDictation(s)).toBe(s);
    expect(correctDictation('')).toBe('');
  });

  it('does not match words that only look alike', () => {
    for (const s of [
      'the gateway api',
      'a lake near a base',
      'data and bricks',
      'machine learning flow',
    ]) {
      expect(correctDictation(s)).toBe(s);
    }
  });

  it('does not match again inside its own output', () => {
    expect(correctDictation('Gateway Gateway')).toBe('Gateway Gateway');
  });
});

describe('plain English stays as it is', () => {
  it.each([
    'We rented a lake house for the weekend.',
    'Gee way to go!',
    'Big data brick wall ahead.',
    'My pie spark plugs are new.',
    'Jenny space is a nice name for a cafe.',
    'Is this the g way to the station?',
    'The lake house had a view of the lake.',
    'He built a wall of red bricks.',
    'The gateway to the city is wide.',
    'Call the API gateway team about the catalog of unity songs.',
    'Pie and spark of joy.',
    'Machine learning flow charts are handy.',
    'I love the lake and the base camp.',
    'She said gee, way to go, champ.',
    'The py file and the spark job both ran.',
    'Unity is a great game engine, and the catalog is long.',
    'Jeannie is my aunt and she likes space movies.',
    'The Genie in the lamp granted three wishes.',
    'Open the data and check the bricks in the report.',
    'River flow rates were low in July.',
  ])('%s', (s) => {
    expect(correctDictation(s)).toBe(s);
  });
});

describe('user words', () => {
  it('parses heard → write lines and counts the bad ones', () => {
    const { rules, invalid } = parseUserWords(
      'sathish → Satish\n# note\n\nfoo -> Foo\nbar => Bar\nno arrow here\n→ nothing\nsame → same\n!!! → x',
    );
    expect(rules).toEqual([
      { heard: 'sathish', write: 'Satish' },
      { heard: 'foo', write: 'Foo' },
      { heard: 'bar', write: 'Bar' },
    ]);
    expect(invalid).toBe(4);
  });

  it('applies user rules and they win over the built-in list', () => {
    const rules = parseUserWords('bricks tea → BricksT\ndata bricks → DataBricks (mine)').rules;
    expect(correctDictation('use data bricks and bricks tea', rules)).toBe(
      'use DataBricks (mine) and BricksT',
    );
  });

  it('matches a multi-word rule across any run of spaces, case-insensitively', () => {
    const rules = parseUserWords('kay eight s → K8s').rules;
    expect(correctDictation('Deploy on KAY  EIGHT S now', rules)).toBe('Deploy on K8s now');
  });

  it('treats special characters in a rule as plain text', () => {
    const rules = parseUserWords('c++ dev → C++ developer\n(x → y').rules;
    expect(correctDictation('a c++ dev. and (x', rules)).toBe('a C++ developer. and y');
    expect(correctDictation('a cdev', rules)).toBe('a cdev');
  });

  it('leaves text alone when no rule matches', () => {
    expect(correctDictation('hello world', parseUserWords('zzz → y').rules)).toBe('hello world');
  });
});

describe('storage', () => {
  it('clears the saved list', () => {
    const map = new Map([[DICTATION_WORDS_STORAGE_KEY, 'a → b']]);
    clearUserWords({ removeItem: (k: string) => void map.delete(k) });
    expect(map.size).toBe(0);
    expect(() =>
      clearUserWords({
        removeItem: () => {
          throw new Error('x');
        },
      }),
    ).not.toThrow();
  });

  it('saves and reads the text, and survives a failing store', () => {
    const map = new Map<string, string>();
    const store = {
      getItem: (k: string) => map.get(k) ?? null,
      setItem: (k: string, v: string) => void map.set(k, v),
    };
    writeUserWordsText('a → b', store);
    expect(map.get(DICTATION_WORDS_STORAGE_KEY)).toBe('a → b');
    expect(readUserWordsText(store)).toBe('a → b');
    const bad = {
      getItem: () => {
        throw new Error('x');
      },
      setItem: () => {
        throw new Error('x');
      },
    };
    expect(readUserWordsText(bad)).toBe('');
    expect(() => writeUserWordsText('a', bad)).not.toThrow();
  });

  it('the corrector reads the list at each call', () => {
    let saved = '';
    const fix = createDictationCorrector(() => saved);
    expect(fix('hello zed')).toBe('hello zed');
    saved = 'zed → Zed';
    expect(fix('hello zed')).toBe('hello Zed');
    expect(
      createDictationCorrector(() => {
        throw new Error('x');
      })('data bricks'),
    ).toBe('Databricks');
  });
});
