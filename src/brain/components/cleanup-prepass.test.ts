import { MEANING_CORPUS, meaningFailures } from '../../test-support/meaning';
import {
  applyDictionary,
  collapseStutters,
  prepass,
  removeFillers,
  spokenNumeralsToDigits,
  wordCount,
} from './cleanup-prepass';

describe('cleanup pre-pass (glossary "pre-pass": fillers, dictionary, numerals)', () => {
  it('[CAP-66a] removes filler sounds and keeps the words around them', () => {
    expect(removeFillers('um ship 1,250 units uh to Lisbon')).toBe('ship 1,250 units to Lisbon');
    expect(removeFillers('so hmm the report, umm, is ready')).toBe('so the report, is ready');
    expect(removeFillers('uhh')).toBe('');
  });

  it('[CAP-66a] a capital filler at the start hands its capital to the next word', () => {
    expect(removeFillers('Um, so I think we ship')).toBe('So I think we ship');
    expect(removeFillers('It works. Uh, then we ship.')).toBe('It works. Then we ship.');
  });

  it('[CAP-66a] removes "um", "ah" and "eh" only for English, where they are not words', () => {
    expect(removeFillers('um the plan', 'en')).toBe('the plan');
    // Portuguese "um" means "a", German "um" means "at".
    expect(removeFillers('tenho um plano', 'pt')).toBe('tenho um plano');
    expect(removeFillers('um 3 Uhr', 'de-DE')).toBe('um 3 Uhr');
    // Sounds that are no word in any language go in every language.
    expect(removeFillers('uhm tenho um plano', 'pt')).toBe('tenho um plano');
  });

  it('[CAP-66a] never removes a filler inside a word', () => {
    expect(removeFillers('the umbrella and the uhlan')).toBe('the umbrella and the uhlan');
    expect(removeFillers('5 mm of rain')).toBe('5 mm of rain');
  });

  it('[CAP-66a] collapses a word said 3 or more times, and keeps a double word', () => {
    expect(collapseStutters('I I I think so')).toBe('I think so');
    expect(collapseStutters('that that is fine')).toBe('that that is fine');
    expect(collapseStutters('10 10 10 rows')).toBe('10 10 10 rows');
  });

  it('[CAP-66a] applies the user dictionary on whole words, case-insensitively', () => {
    const dict = [
      { from: 'data bricks', to: 'Databricks' },
      { from: 'unity catalogue', to: 'Unity Catalog' },
    ];
    expect(applyDictionary('open Data Bricks and unity catalogue', dict)).toBe(
      'open Databricks and Unity Catalog',
    );
    expect(applyDictionary('metadata bricks', dict)).toBe('metadata bricks');
    expect(applyDictionary('no entries', [])).toBe('no entries');
    // A dictionary entry with regex characters matches literally.
    expect(applyDictionary('use c++ here', [{ from: 'c++', to: 'C++' }])).toBe('use C++ here');
  });

  it('[CAP-66a] writes spoken cardinals of 10 or more, or of 2 or more words, as digits', () => {
    expect(spokenNumeralsToDigits('twenty five users')).toBe('25 users');
    expect(spokenNumeralsToDigits('twenty-five users')).toBe('25 users');
    expect(spokenNumeralsToDigits('one hundred and twenty rows')).toBe('120 rows');
    expect(spokenNumeralsToDigits('three thousand four hundred units')).toBe('3400 units');
    expect(spokenNumeralsToDigits('wait ten minutes')).toBe('wait 10 minutes');
    expect(spokenNumeralsToDigits('Twelve deals')).toBe('12 deals');
  });

  it('[CAP-66a] keeps a lone small number word, digit read-outs and mixed numbers as they are', () => {
    expect(spokenNumeralsToDigits('the offices share one budget')).toBe(
      'the offices share one budget',
    );
    expect(spokenNumeralsToDigits('dial one two three')).toBe('dial one two three');
    expect(spokenNumeralsToDigits('the budget is 40 thousand')).toBe('the budget is 40 thousand');
    expect(spokenNumeralsToDigits('a hundred reasons')).toBe('a hundred reasons');
  });

  it('[CAP-66a] [CAP-68] never converts a number next to "point", on either side', () => {
    for (const t of [
      'revenue was two point five million dollars',
      'one point two thousand rows',
      'we hit nine point five thousand users',
      'version two point eleven',
      'twenty point five percent',
    ]) {
      expect(spokenNumeralsToDigits(t), t).toBe(t);
    }
  });

  it('[CAP-66a] [CAP-68] never converts a number run before an ordinal', () => {
    for (const t of [
      'the twenty first of june',
      'twenty first place',
      'one hundred and first',
      'the thirty second floor',
      'the two hundredth order',
    ]) {
      expect(spokenNumeralsToDigits(t), t).toBe(t);
    }
  });

  it('[CAP-66a] [CAP-68] the self-check counts ordinal words', () => {
    // A dictionary entry that writes an ordinal as digits is a number change: the raw text stays.
    expect(prepass('um take the first row', { dictionary: [{ from: 'first', to: '1st' }] })).toBe(
      'um take the first row',
    );
  });

  it('[CAP-66a] [CAP-68] keeps a comma list, a number after a currency word, and a run split by a filler in words', () => {
    expect(spokenNumeralsToDigits('twenty, thirty, forty')).toBe('twenty, thirty, forty');
    expect(spokenNumeralsToDigits('it costs a dollar fifty')).toBe('it costs a dollar fifty');
    expect(spokenNumeralsToDigits('it costs a euro twenty')).toBe('it costs a euro twenty');
    expect(prepass('twenty uh five rows')).toBe('twenty five rows');
    expect(prepass('twenty uh, five rows')).toBe('twenty five rows');
    // A plain run still converts.
    expect(prepass('um twenty five rows')).toBe('25 rows');
  });

  it('[CAP-66a] [CAP-68] never converts a number in a range or a list', () => {
    for (const t of [
      'we need three to four hundred servers',
      'three hundred to four hundred servers',
      'between twenty and thirty rows',
      'twenty or thirty more',
      'from 5 to twenty users',
      'twenty through forty',
    ]) {
      expect(spokenNumeralsToDigits(t), t).toBe(t);
    }
  });

  it('[CAP-66a] [CAP-68] never collapses repeated number words', () => {
    expect(collapseStutters('call me at five five five one two one two')).toBe(
      'call me at five five five one two one two',
    );
    expect(prepass('call me at five five five one two one two')).toBe(
      'call me at five five five one two one two',
    );
  });

  it('[CAP-66a] [CAP-68] a pre-pass that would change a number keeps the raw text', () => {
    // A dictionary entry that writes a number is a number change: the raw text stays.
    const dict = [{ from: 'q three', to: 'Q3' }];
    expect(prepass('um we beat the q three targets', { dictionary: dict })).toBe(
      'um we beat the q three targets',
    );
    // A dictionary entry with no number still applies.
    expect(
      prepass('um open data bricks', { dictionary: [{ from: 'data bricks', to: 'Databricks' }] }),
    ).toBe('open Databricks');
  });

  it('[CAP-66a] runs fillers, dictionary and numerals, and is deterministic and idempotent', () => {
    const dict = [{ from: 'data bricks', to: 'Databricks' }];
    const raw = 'um so so so we moved twenty five tables to uh data bricks';
    const once = prepass(raw, { dictionary: dict });
    expect(once).toBe('so we moved 25 tables to Databricks');
    expect(prepass(raw, { dictionary: dict })).toBe(once);
    expect(prepass(once, { dictionary: dict })).toBe(once);
    expect(prepass('   ')).toBe('');
  });

  it('[CAP-66a] [AT-05] the pre-pass alone keeps the meaning of every corpus item', () => {
    for (const x of MEANING_CORPUS) {
      expect(meaningFailures(x, { text: prepass(x.raw), usedRaw: false }), x.id).toEqual([]);
    }
  });

  it('[CAP-66b] counts words for the 4-word rule', () => {
    expect(wordCount('Send it now')).toBe(3);
    expect(wordCount('  send it  now please ')).toBe(4);
    expect(wordCount('')).toBe(0);
    expect(wordCount('main.finance.invoices rows')).toBe(2);
  });
});
