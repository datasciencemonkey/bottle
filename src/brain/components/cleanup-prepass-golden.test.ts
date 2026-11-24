/** The golden set of the dictation pre-pass (CAP-66a, ADR 0122): what it cleans and what it keeps. */
import { prepass } from './cleanup-prepass';

/** [input, output]: the pre-pass must clean the input to the output. */
const CLEANS: ReadonlyArray<readonly [string, string]> = [
  // fillers
  ['um I think we should ship', 'I think we should ship'],
  ['uh the report is ready', 'The report is ready'],
  ['the report is, um, ready', 'The report is ready'],
  ['er the plan changed', 'The plan changed'],
  ['ah okay that works', 'Okay that works'],
  ['hmm let me check the deck', 'Let me check the deck'],
  ['Um, so we ship on Friday.', 'So we ship on Friday.'],
  ['uhm so we ship', 'So we ship'],
  // discourse fillers in a clear position
  ['It was, you know, a long week.', 'It was a long week.'],
  ['You know, we should ship it.', 'We should ship it.'],
  ['We should ship it, you know.', 'We should ship it.'],
  ['I mean, we should ship it.', 'We should ship it.'],
  ['The demo went well. I mean, the demo went fine.', 'The demo went well. The demo went fine.'],
  ['It was, like, really huge.', 'It was really huge.'],
  ['Like, I do not know what happened.', 'I do not know what happened.'],
  ['we shipped it, like', 'We shipped it'],
  // stutters and false starts
  ['the the report is ready', 'The report is ready'],
  ['I I think so', 'I think so'],
  ['I I I think so', 'I think so'],
  ['we need to to ship it', 'We need to ship it'],
  ['I want to I want to go home', 'I want to go home'],
  ['I think I think we should ship', 'I think we should ship'],
  ['wh- what is the plan', 'What is the plan'],
  ['th- the report is ready', 'The report is ready'],
  // spacing, punctuation, capitals
  ['  we   ship  on friday  ', 'We ship on friday'],
  ['we ship on friday . then we rest', 'We ship on friday. Then we rest'],
  ['it works. then we ship. and we rest', 'It works. Then we ship. And we rest'],
  ['the build passed , the tests passed', 'The build passed, the tests passed'],
  ['i think so', 'I think so'],
  // M4: a filler next to a number or a yes/no word stays; L4 to L6
  ['give me five, like, six apples', 'Give me five, like, six apples'],
  ['give me 5, like, 6 apples', 'Give me 5, like, 6 apples'],
  ['I said no, I mean, yes', 'I said no, I mean, yes'],
  ['it is not, you know, done', 'It is not, you know, done'],
  ['it was, like, um, huge', 'It was huge'],
  ["i think i'm done and i will go", "I think I'm done and I will go"],
  ['we ship, um.', 'We ship'],
  // combined
  ['um so uh the the report is, you know, ready', 'So the report is ready'],
  ['uh I mean, we should, like, ship it.', 'We should ship it.'],
  // numerals stay what they are
  ['um ship 1,250 units uh to Lisbon', 'Ship 1,250 units to Lisbon'],
  ['we need 3.5 hours', 'We need 3.5 hours'],
];

/** The pre-pass must give the input back as it is (or only with a first-letter capital). */
const KEEPS: readonly string[] = [
  // "like" as a verb or a comparison
  'I like pizza.',
  'It looks like rain.',
  'I would like to order three items.',
  // "like" before a number or in a list
  'Pick like 3 items from the list.',
  'We stock fruit like apples and pears.',
  // "you know" and "I mean" as real phrases
  'You know the answer.',
  'Do you know the answer?',
  'I know what you mean.',
  'I mean it.',
  'What do you mean by that?',
  // double words that are real
  'He said that that was fine.',
  'I had had enough.',
  'Bye bye.',
  'It is very very good.',
  // names, places, entities
  'We flew to New York New York.',
  'Ask Sam Lee about the Q3 plan.',
  'Contoso and EMEA are on the list.',
  // numbers
  'Order 1,250 units on 2026-11-24.',
  'Call 555 0100 before 10:30.',
  'Revenue grew 3.5 percent.',
  // code, URLs, email, quotes
  'Run `um the the` in the shell.',
  'Open https://example.com/um/uh/the-the now.',
  'Mail sam.um@example.com about it.',
  'He said "um the the like, you know" and left.',
  'Call main.finance.invoices today.',
  // a huge text and a text with our mark character stay as they are
  'a\uE001 b um',
  // abbreviations and mixed-case words stay
  'Use e.g. the iPhone app.',
  'Wait for the eBay order.',
  // a plain clean sentence
  'The deck is attached and the numbers follow tomorrow.',
];

describe('the dictation pre-pass golden set', () => {
  it.each(CLEANS)('[CAP-66a] cleans %j', (input, expected) => {
    expect(prepass(input)).toBe(expected);
  });

  it.each(KEEPS)('[CAP-66a] [CAP-68] keeps %j', (input) => {
    expect(prepass(input)).toBe(input);
  });

  it('[CAP-66a] holds at least 40 golden cases', () => {
    expect(CLEANS.length + KEEPS.length).toBeGreaterThanOrEqual(40);
  });

  it('[CAP-66a] is idempotent on every golden case', () => {
    for (const [input] of CLEANS) {
      const once = prepass(input);
      expect(prepass(once), input).toBe(once);
    }
  });

  it('[CAP-66a] a text over 8000 characters stays as it is, fast', () => {
    const big = 'um the the report '.repeat(600);
    expect(big.length).toBeGreaterThan(8000);
    expect(prepass(big)).toBe(big.trim());
    const at = 'a@'.repeat(3000);
    const t0 = Date.now();
    prepass(at);
    expect(Date.now() - t0).toBeLessThan(500);
  });

  it('[CAP-66a] gives no text from empty and blank input', () => {
    expect(prepass('')).toBe('');
    expect(prepass('  \n ')).toBe('');
  });

  it('[CAP-66a] [CAP-68] never changes the numbers of any golden case', () => {
    const nums = (s: string) => (s.match(/\d+(?:[.,]\d+)*/g) ?? []).join(' ');
    for (const [input] of CLEANS) expect(nums(prepass(input)), input).toBe(nums(input));
    for (const input of KEEPS) expect(nums(prepass(input)), input).toBe(nums(input));
  });
});
