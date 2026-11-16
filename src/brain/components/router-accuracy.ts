/**
 * Intent-accuracy report for the router step (#57, RTR-01, AT-06).
 *
 * Runs a golden utterance set through a classifier and counts the intents it gets right. The
 * report holds golden IDs and counts only, never utterance text. RTR-01 needs >= 90 % on a real
 * router model. TODO(#56): SP-13 runs the golden set on the live router models; until then the
 * only reports come from the `model_invoke` test double and say so.
 */
import type { CaptureMode, Intent } from '../../shared/interfaces';
import { routerJsonSchema } from '../../shared/router-schema';

export const INTENT_ACCURACY_TARGET = 0.9;

const INTENTS = routerJsonSchema.schema.properties.intent.enum;
const MODES: readonly CaptureMode[] = ['ptt', 'hands_free', 'session'];

export interface GoldenItem {
  id: string;
  text: string;
  captureMode: CaptureMode;
  speakerTrack: 'you' | 'others';
  expected: Intent;
}

/** Parses a golden-set file (`tests/fixtures/router-golden/utterances.json`). Throws on a bad item. */
export function parseGoldenSet(json: unknown): GoldenItem[] {
  const doc = json as { version?: unknown; items?: unknown };
  if (doc?.version !== 1 || !Array.isArray(doc.items))
    throw new Error('golden set: need version 1 and items');
  return doc.items.map((raw, i) => {
    const it = raw as Partial<Record<keyof GoldenItem, unknown>>;
    const ok =
      typeof it.id === 'string' &&
      typeof it.text === 'string' &&
      MODES.includes(it.captureMode as CaptureMode) &&
      (it.speakerTrack === 'you' || it.speakerTrack === 'others') &&
      (INTENTS as readonly unknown[]).includes(it.expected);
    if (!ok) throw new Error(`golden set: item ${i} is not valid`);
    return it as GoldenItem;
  });
}

type Counts = { total: number; correct: number };

export interface IntentAccuracyReport {
  total: number;
  correct: number;
  accuracy: number;
  target: number;
  meetsTarget: boolean;
  perIntent: Record<Intent, Counts>;
  /** confusion[expected][got] */
  confusion: Record<Intent, Record<Intent, number>>;
  missIds: string[];
}

function byIntent<T>(f: () => T): Record<Intent, T> {
  return Object.fromEntries(INTENTS.map((i) => [i, f()])) as Record<Intent, T>;
}

/** Classifies each golden item in order and counts the results. */
export async function measureIntentAccuracy(
  items: readonly GoldenItem[],
  classify: (item: GoldenItem) => Promise<Intent>,
): Promise<IntentAccuracyReport> {
  const perIntent = byIntent<Counts>(() => ({ total: 0, correct: 0 }));
  const confusion = byIntent(() => byIntent(() => 0));
  const missIds: string[] = [];
  let correct = 0;
  for (const item of items) {
    const got = await classify(item);
    perIntent[item.expected].total++;
    confusion[item.expected][got]++;
    if (got === item.expected) {
      correct++;
      perIntent[item.expected].correct++;
    } else missIds.push(item.id);
  }
  const accuracy = items.length ? correct / items.length : 0;
  return {
    total: items.length,
    correct,
    accuracy,
    target: INTENT_ACCURACY_TARGET,
    meetsTarget: items.length > 0 && accuracy >= INTENT_ACCURACY_TARGET,
    perIntent,
    confusion,
    missIds,
  };
}

const pct = (x: number) => `${(x * 100).toFixed(1)} %`;

export interface ReportMeta {
  /** `double` = the `model_invoke` test double; `live` = a real router model (SP-13). */
  source: 'double' | 'live';
  model: string;
  date: string;
}

/** The report as markdown: counts, per-intent rows, confusion table and miss IDs. No utterance text. */
export function formatIntentAccuracyReport(r: IntentAccuracyReport, meta: ReportMeta): string {
  const lines = [
    '# Router intent accuracy (RTR-01, AT-06)',
    '',
    `- Source: ${meta.source === 'double' ? 'model_invoke test double' : 'live router model'}`,
    `- Model: ${meta.model}`,
    `- Date: ${meta.date}`,
    `- Accuracy: ${pct(r.accuracy)} (${r.correct} of ${r.total}); target ${pct(r.target)}: ${r.meetsTarget ? 'met' : 'not met'}`,
  ];
  if (meta.source === 'double')
    lines.push(
      '',
      '> This report comes from a test double. It is not a model result. The live run on the router models is pending on #56 (SP-13).',
    );
  lines.push('', '| Intent | Total | Correct | Accuracy |', '|---|---|---|---|');
  for (const i of INTENTS) {
    const c = r.perIntent[i];
    lines.push(
      `| ${i} | ${c.total} | ${c.correct} | ${c.total ? pct(c.correct / c.total) : '-'} |`,
    );
  }
  lines.push(
    '',
    `| Expected \\ Got | ${INTENTS.join(' | ')} |`,
    `|---|${INTENTS.map(() => '---').join('|')}|`,
  );
  for (const e of INTENTS)
    lines.push(`| ${e} | ${INTENTS.map((g) => r.confusion[e][g]).join(' | ')} |`);
  lines.push('', `Misses (golden IDs): ${r.missIds.length ? r.missIds.join(', ') : 'none'}`, '');
  return lines.join('\n');
}
