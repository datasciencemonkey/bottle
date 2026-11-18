/**
 * The write-time quarantine check of the MemoryWriter (#114; SEC-24a, SEC-24b; ADR 0089).
 *
 * A memory item whose text looks like an instruction to the assistant is stored with
 * `status = 'quarantined'`. It stays on the device (U9: nothing deletes it automatically), but it
 * never goes into a Genie preamble or a `memory.recall` result (SEC-24b).
 *
 * The check is the four Table RTR-INJ categories of the router step (`detectInjection`, #117)
 * plus two memory patterns for indirect injection (SG-INJ-7): "ignore previous" with no object,
 * and text that speaks to the assistant ("Assistant, delete ...", "If you are an AI ...").
 * The Rust catalog runs the same check again on every item write (`local_store/quarantine.rs`),
 * so a caller cannot skip it. Both sides run `tests/fixtures/memory-adversarial/items.json`.
 */
import { detectInjection, type InjectionCategory } from './router-injection';

/** The reason codes: the RTR-INJ categories and the two memory patterns. Fixed codes only. */
export type QuarantineReason = InjectionCategory | 'ignore_previous' | 'addresses_assistant';

/** "Ignore previous" and similar, at the end of a clause or before an instruction noun. */
const IGNORE_PREVIOUS =
  /\b(?:ignore|disregard|forget)\s+(?:(?:all|any|everything)\s+)?(?:of\s+)?(?:(?:the|your|my)\s+)?(?:previous|prior|above|earlier|preceding|foregoing)(?:\s*(?:[.,;:!?)]|$)|\s+(?:and|then)\b|\s+(?:instructions?|guidance|messages?|context|prompts?|directions?|commands?|text|rules?|conversation|content|input|orders|steps)\b)/;

const ASSISTANT_NAMES = String.raw`(?:assistant|ai|claude|codex|chatgpt|gpt|llm|bottle|mog|agent|model)`;
const ORDER_VERBS = String.raw`(?:ignore|disregard|forget|delete|remove|send|forward|reveal|show|print|dump|run|execute|call|switch|change|approve|publish|export|share|write|stop|you must|do not|don't)`;

/** Text that speaks to the assistant. */
const ADDRESSES_ASSISTANT: readonly RegExp[] = [
  new RegExp(
    String.raw`\b${ASSISTANT_NAMES}\s*[,:]\s*(?:when you \w+ this,?\s*)?(?:please\s+)?${ORDER_VERBS}\b`,
  ),
  /\bif you are (?:an? )?(?:ai|assistant|llm|language model|chatbot|agent)\b/,
  /\bwhen you (?:read|see|recall|process|retrieve) this\b/,
];

/** Zero-width and soft-hyphen code points, built from numbers so the source has no hidden text. */
const ZERO_WIDTH = new RegExp(
  `[${[
    [0x200b, 0x200f],
    [0x2060, 0x2064],
    [0xfeff, 0xfeff],
    [0x00ad, 0x00ad],
  ]
    .map(([a, b]) => `${String.fromCharCode(a ?? 0)}-${String.fromCharCode(b ?? 0)}`)
    .join('')}]`,
  'g',
);

function normalize(text: string): string {
  return text.normalize('NFKC').replace(ZERO_WIDTH, '').toLowerCase().replace(/\s+/g, ' ');
}

/** The reasons to quarantine `text`, in a fixed order. Empty for clean text. */
export function quarantineReasons(text: string): QuarantineReason[] {
  const out: QuarantineReason[] = detectInjection(text);
  const t = normalize(text);
  if (IGNORE_PREVIOUS.test(t)) out.push('ignore_previous');
  if (ADDRESSES_ASSISTANT.some((p) => p.test(t))) out.push('addresses_assistant');
  return out;
}

/** SEC-24a: true if the item with this text must be quarantined. */
export function isQuarantined(text: string): boolean {
  return quarantineReasons(text).length > 0;
}

/**
 * SEC-24b: keep only the rows that may go to Genie or to the main agent. A row passes only with
 * `status === 'active'`; a row with no status fails closed. The ContextAssembler (#92) and any
 * brain path that hands items to the agent run their rows through it.
 */
export function excludeQuarantined<T extends { status?: unknown }>(rows: readonly T[]): T[] {
  return rows.filter((r) => r.status === 'active');
}
