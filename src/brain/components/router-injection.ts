/**
 * The client-side Table RTR-INJ check of the router step (#117; RTR-03b, RTR-14, ADR 0056).
 *
 * `detectInjection` matches an utterance against the four RTR-INJ categories. `applyInjectionGuard`
 * runs after the GL-7.2 field rules: a match, or a model flag, sets `injection_suspected=true` and
 * removes every field that could lead to a send, a command, a hop or a split. A flagged decision
 * never may send (`maySend`) and shows a warning chip (`routerChips`).
 *
 * The guard only removes actions. It never starts capture, approves context, publishes a Page,
 * switches workspace or calls Genie. The chip has fixed text and never holds the utterance.
 */
import type { Intent, RouterDecision } from '../../shared/interfaces';

/** The categories of Table RTR-INJ (requirements 5.13). */
export const RTR_INJ_CATEGORIES = [
  'change_rules',
  'show_memory',
  'send_elsewhere',
  'role_tag',
] as const;

export type InjectionCategory = (typeof RTR_INJ_CATEGORIES)[number];

// Optional filler words between a verb and its object ("ignore all of the previous ...").
const FILL = String.raw`(?:(?:all|any|every|of|the|your|my|its|their|these|those|this|that|previous|prior|above|earlier|current|existing|stored|saved|whole|entire|full|raw|out|me|us|everything|in|from)\s+){0,5}`;
const SEND_VERB = String.raw`\b(?:send|sends|sending|post|upload|forward|copy|export|share|email|e-mail|transfer|exfiltrate|leak|push|sync|move|beam|transmit)\b`;
const URL = String.raw`(?:https?:\/\/|\bwww\.|\b[a-z0-9-]+(?:\.[a-z0-9-]+)*\.(?:com|net|org|io|ai|dev|app|cloud|xyz|ru|cn|me|co|info|biz|site|test|example|invalid|local)\b|[a-z0-9._%+-]+@[a-z0-9-]+\.)`;

/** The patterns of each category. Every pattern runs on `normalize(text)`. */
const PATTERNS: Record<InjectionCategory, readonly RegExp[]> = {
  // A request to change rules or settings.
  change_rules: [
    new RegExp(
      String.raw`\b(?:ignore|disregard|forget|override|overwrite|bypass|skip|drop|break|suspend)\s+${FILL}(?:instructions?|rules?|prompts?|system prompt|guidelines?|polic(?:y|ies)|guardrails?|restrictions?|safeguards?|filters?|checks?)\b`,
    ),
    /\b(?:new|updated|different|additional|revised)\s+(?:instructions?|rules?|system prompt|polic(?:y|ies))\b/,
    new RegExp(
      String.raw`\b(?:change|update|modify|edit|reset|turn on|turn off|switch on|switch off|enable|disable|unlock|remove|add to)\s+${FILL}(?:rules?|settings?|configuration|config|polic(?:y|ies)|permissions?|allow ?list|allowed hosts|auto[ -]?ask|consent(?: check)?|guardrails?|tools?|safety|injection[ _]suspected)\b`,
    ),
    new RegExp(
      String.raw`\b(?:turn on|turn off|switch on|switch off|enable|disable|activate|unlock)\s+${FILL}(?:context|memory)\b`,
    ),
    /\b(?:you are now|you're now|act as|pretend (?:to be|you are|you're)|roleplay as|developer mode|jailbreak|dan mode|god mode|admin mode)\b/,
  ],
  // A request to show memory (or the prompt, secrets and tokens it guards).
  show_memory: [
    new RegExp(
      String.raw`\b(?:show|tell|read|list|reveal|print|dump|give|repeat|recite|output|display|expose|leak|export|spell)\s+${FILL}(?:memory|memories|system prompt|secrets?|tokens?|credentials?|passwords?|api keys?)\b`,
    ),
    /\b(?:show|tell|read|list|reveal|print|dump|give|repeat|recite|output|display|expose|spell)\s+(?:me\s+|us\s+)?(?:all\s+)?(?:of\s+)?your\s+(?:instructions|rules|prompt|settings|configuration)\b/,
    /\bwhat(?:'s| is| are)? (?:in|inside|stored in) (?:your|the|my) (?:memory|memories|prompt|system prompt)\b/,
  ],
  // A request to send data to another workspace or URL.
  send_elsewhere: [
    new RegExp(
      String.raw`${SEND_VERB}.{0,80}?\b(?:to|into|at|on|onto)\b.{0,40}?(?:\b(?:another|other|different|external|outside|personal|private|third[- ]party|foreign|my own)\s+(?:workspace|server|host|site|account|endpoint|address|domain|url|bucket)|\b(?:url|link|webhook|endpoint|server|address|domain|website|pastebin)\b|${URL})`,
    ),
  ],
  // Role tags: `system:`, `assistant:`, `<|im_start|>` and similar.
  role_tag: [
    /(?:^|[\s"'([{>|])(?:system|assistant|developer)\s*:/,
    /<\|[a-z_]{2,20}\|>/,
    /<\/?\s*(?:system|assistant|developer|user|instructions?|utterance|router_input|prompt)\s*>/,
    /\[\/?\s*(?:inst|sys|system)\s*\]/,
    /<<\/?sys>>/,
    // The guard's own answer block. The open bracket may also be a numeric or named entity.
    /(?:<|&#0*60;|&#x0*3c;|&lt;)\s*\/?\s*genie_answer\b/,
    /#{2,}\s*(?:system|instruction|instructions)\b/,
  ],
};

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

/** NFKC folds full-width forms; zero-width characters go; case and spaces fold. */
function normalize(text: string): string {
  return text
    .normalize('NFKC')
    .replace(/[‹〈❮˂ᐸ]/g, '<')
    .replace(ZERO_WIDTH, '')
    .toLowerCase()
    .replace(/\s+/g, ' ');
}

/** The RTR-INJ categories that `text` matches, in table order. */
export function detectInjection(text: string): InjectionCategory[] {
  const t = normalize(text);
  return RTR_INJ_CATEGORIES.filter((c) => PATTERNS[c].some((p) => p.test(t)));
}

/** Intents that ask or command. A flagged decision with one of them becomes `context`. */
const ACTING: ReadonlySet<Intent> = new Set(['question', 'memory_question', 'command']);

/**
 * Applies RTR-14 to a decision that passed the field rules. Returns the decision, the matched
 * categories, and whether the guard flagged it (a match or the model's own flag).
 */
export function applyInjectionGuard(
  d: RouterDecision,
  text: string,
): { decision: RouterDecision; categories: InjectionCategory[]; flagged: boolean } {
  const categories = detectInjection(text);
  const flagged = categories.length > 0 || d.injection_suspected;
  if (!flagged) return { decision: d, categories, flagged };
  return {
    decision: {
      ...d,
      intent: ACTING.has(d.intent) ? 'context' : d.intent,
      genie_question: '',
      target_profile_hint: '',
      part_questions: [],
      part_profile_hints: [],
      command: 'none',
      command_arg: '',
      injection_suspected: true,
    },
    categories,
    flagged,
  };
}

/**
 * Whether a router decision may go on to the send preview (PTT, 1.5 s, cancelable). It never
 * sends itself: the user action of the preview gate (#142) still decides.
 * The #59 route and the ask gate (`ask-gate.ts`, #142) apply its injection rule to every ask.
 */
export function maySend(d: RouterDecision): boolean {
  if (d.injection_suspected) return false;
  if (d.intent !== 'question' && d.intent !== 'memory_question') return false;
  return d.genie_question.trim() !== '' || d.cleaned_text.trim() !== '';
}

/** The warning chip of RTR-14. Fixed text; it never holds the utterance. */
export const INJECTION_WARNING_CHIP: Readonly<{ id: 'injection_warning'; label: string }> =
  Object.freeze({
    id: 'injection_warning',
    label: 'Warning: this sounded like an instruction to Bottle. Nothing was sent.',
  });

/**
 * The chips that a router decision adds to the card (`CardUpdate.chips` shape).
 * The ask gate (#142) shows `INJECTION_WARNING_CHIP` for a flagged ask.
 */
export function routerChips(d: RouterDecision): { id: string; label: string }[] {
  return d.injection_suspected ? [{ ...INJECTION_WARNING_CHIP }] : [];
}
