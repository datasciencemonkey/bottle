/**
 * The RTR-09b grounding check (#135, ADR 0065): the numbers and entities of a router
 * `genie_question` must be in the cleaned text. The router field rules (`router-step.ts`) and
 * the cleanup merge (`router-parallel.ts`) both use it.
 */

/** The numbers in `s` ("1,200", "3.5", "2026"). */
export function numbersOf(s: string): string[] {
  return s.match(/\d+(?:[.,]\d+)*/g) ?? [];
}

/**
 * The entities in `s`, lowercase: words that start with a capital letter or hold a digit next to
 * a letter (`Contoso`, `EMEA`, `Q3`). The first word of a sentence and the pronoun "I" are not
 * entities (ADR 0065).
 */
export function entitiesOf(s: string): string[] {
  const out: string[] = [];
  for (const sentence of s.split(/[.?!]+\s*/)) {
    const words = sentence.match(/[\p{L}\p{N}][\p{L}\p{N}'&-]*/gu) ?? [];
    words.forEach((w, i) => {
      if (i === 0 || w === 'I') return;
      const capital = /^\p{Lu}/u.test(w);
      const mixed = /\p{L}/u.test(w) && /\p{N}/u.test(w);
      if (capital || mixed) out.push(w.toLowerCase());
    });
  }
  return out;
}

/** The lowercase words of `s`, for the entity check. */
function wordsOf(s: string): Set<string> {
  return new Set((s.match(/[\p{L}\p{N}][\p{L}\p{N}'&-]*/gu) ?? []).map((w) => w.toLowerCase()));
}

/** Whether every number and entity of `question` is in `cleaned` (RTR-09b). */
export function groundedIn(question: string, cleaned: string): boolean {
  const nums = new Set(numbersOf(cleaned));
  if (numbersOf(question).some((n) => !nums.has(n))) return false;
  const words = wordsOf(cleaned);
  return entitiesOf(question).every((e) => words.has(e));
}
