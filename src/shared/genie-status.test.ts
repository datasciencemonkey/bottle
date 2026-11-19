import {
  ANSWER_MAX_MS,
  ANSWER_MIN_MS,
  answerShowMs,
  genieBlurbId,
  genieStage,
  isGenieBlurbId,
  opIdOfGenieBlurb,
  timerText,
} from './genie-status';

describe('Genie status line (#573)', () => {
  it('[G-17b] [UX-16a] maps the newest step to a fixed stage line and never shows the step text', () => {
    // Fake steps with the prefixes of the live replies (2026-11-19).
    expect(genieStage(['Running SQL: SELECT secret_col FROM fake.t'])).toBe('Running the query');
    expect(genieStage(['Running SQL: SELECT 1', 'Query result: | a |\n| 1 |'])).toBe(
      'Reading the results',
    );
    expect(genieStage(['Thinking: Get totals.'])).toBe('Thinking it through');
    expect(genieStage(['Writing SQL for fake.t'])).toBe('Writing the query');
    expect(genieStage(['Searching fake tables'])).toBe('Finding the data');
  });

  it('[G-17a] keeps the shown line for no steps or an unknown step', () => {
    expect(genieStage(null)).toBeUndefined();
    expect(genieStage([])).toBeUndefined();
    expect(genieStage(['fake_catalog.fake_schema'])).toBeUndefined();
  });

  it('[G-17a] shows the timer after 3 s, and "Working… m:ss" after 60 s', () => {
    expect(timerText(2_999)).toBeUndefined();
    expect(timerText(3_000)).toBe('0:03');
    expect(timerText(59_999)).toBe('0:59');
    expect(timerText(64_000)).toBe('Working… 1:04');
  });

  it('keeps an answer for 4 s + 0.25 s per word, from 6 s to 20 s', () => {
    expect(answerShowMs('Yes.')).toBe(ANSWER_MIN_MS);
    expect(answerShowMs(Array.from({ length: 20 }, () => 'word').join(' '))).toBe(9_000);
    expect(answerShowMs('## Head\n\n**12** | - |')).toBe(ANSWER_MIN_MS);
    expect(answerShowMs(Array.from({ length: 500 }, () => 'w').join(' '))).toBe(ANSWER_MAX_MS);
  });

  it('names the Genie blurb of an operation', () => {
    const id = genieBlurbId('op-1');
    expect(isGenieBlurbId(id)).toBe(true);
    expect(isGenieBlurbId('ask:u1')).toBe(false);
    expect(opIdOfGenieBlurb(id)).toBe('op-1');
    expect(opIdOfGenieBlurb('ask:u1')).toBeUndefined();
  });
});
