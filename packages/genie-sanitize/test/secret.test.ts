import { describe, expect, it } from 'vitest';
import { findSecret, prepareAsk } from '../src/index.ts';

// Obviously fake values, built at run time so that no literal secret sits in the repo.
const FAKE = {
  dapi: 'dapi' + '0'.repeat(32),
  jwt: 'eyJ' + 'fakeheader' + '.' + 'eyJ' + 'fakebody' + '.sig',
  akia: 'AKIA' + 'A'.repeat(16),
  ghp: 'ghp_' + 'x'.repeat(36),
  pem: '-----' + 'BEGIN PRIVATE KEY-----',
  dose: 'dose' + '0'.repeat(32),
  password: 'password' + '=hunter-fake',
};

describe('findSecret and the SECRET_IN_QUESTION block', () => {
  for (const [name, value] of Object.entries(FAKE)) {
    it(`[G-36] blocks a question that contains a ${name} pattern with SECRET_IN_QUESTION`, () => {
      expect(findSecret(`why does ${value} fail`)).toBeDefined();
      const r = prepareAsk({ question: `why does ${value} fail?` });
      expect(r.ok).toBe(false);
      if (!r.ok) {
        expect(r.code).toBe('SECRET_IN_QUESTION');
        expect(r.userText).not.toContain(value);
      }
    });
  }

  it('[G-36] blocks a secret that is split by zero-width characters', () => {
    const hidden = 'dapi' + '\u200B' + '0'.repeat(32);
    const r = prepareAsk({ question: `token ${hidden}` });
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.code).toBe('SECRET_IN_QUESTION');
  });

  it('[G-36] blocks a secret that hides inside a markdown link or an HTML attribute', () => {
    for (const q of [`see [here](https://x.invalid/?t=${FAKE.ghp})`, `<a title="${FAKE.akia}">x</a>`]) {
      const r = prepareAsk({ question: q });
      expect(r.ok).toBe(false);
      if (!r.ok) expect(r.code).toBe('SECRET_IN_QUESTION');
    }
  });

  it('[G-36] lets a question without a secret pattern go', () => {
    expect(findSecret('What were total sales by region in 2025?')).toBeUndefined();
    expect(findSecret('dapi tokens are short-lived')).toBeUndefined();
    expect(prepareAsk({ question: 'What were total sales by region in 2025?' }).ok).toBe(true);
  });

  it('[G-36] returns the pattern name and never the matched text', () => {
    const name = findSecret(`x ${FAKE.dapi}`);
    expect(name).toBe('databricks_pat');
    expect(name).not.toContain('0'.repeat(32));
  });
});
