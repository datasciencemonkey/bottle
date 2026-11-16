/**
 * AT-09 "Show every response outcome" (#234), section 15 (device). Pass: the blurb keeps the
 * source meaning and status; key numbers are verbatim; the expanded card shows all sections;
 * banned words never show.
 *
 * The card parts ship, so they run here with the outcomes of the #229 Genie One MCP double
 * (completed, incomplete, failed, truncated, needs-input): the blurb stack (UX-11a, UX-11b,
 * UX-11c, UX-12; #85), the expanded sections and the 50-row table cap (UX-18, the display part
 * of G-24b; #89), the chip row that acts only on a click (G-38b, UX-20c), the notification
 * service (UX-26a, UX-26b, UX-26c; #95) and the MoodMapper (UX-01a). The event normaliser that
 * turns a poll into an AskEvent is #33, so `finalPoll` below is test glue that reads the double
 * directly; it is not the bridge. The blurb lines, key numbers and templates (#86), the banned
 * words and the 2 s throttle (#88), the long-ask rules (#149), the on-demand table (#150) and
 * the exchange row (#40) are pending. UX-01b (views never set the mood) is the inspection test
 * `src/views/mood-write.test.ts`. The visual check on a device is a person step
 * (docs/tests/AT-GENIE-MCP.md, part AT-09).
 */
import type { MogId } from '../shared/interfaces';
import { BlurbStack } from '../brain/blurb-stack';
import { buildSections, PREVIEW_ROWS, type Table } from '../brain/card-sections';
import { ChipRow } from '../brain/card-chips';
import { createSimClock } from '../brain/clock';
import type { Scheduler } from '../brain/card-stack';
import { mapMood, type MoodSources, type RequestPhase } from '../brain/components/mood-mapper';
import { createNotificationService, type OsNotification } from '../brain/notification-service';
import {
  createGenieOneMcpDouble,
  scenarios,
  type PollStatus,
  type QueryResult,
  type ScriptedResponse,
} from '../test-support/genie-one-mcp';

const HOST = 'example.cloud.databricks.com';
const WSID = '1234567890';
const MOG_A = '00000000-0000-4000-8000-0000000000c9' as MogId;
const MOG_B = '00000000-0000-4000-8000-0000000000ca' as MogId;

interface Final {
  status: PollStatus;
  final_answer: string | null;
  deep_link: string | null;
  query_items: { item_id: string; sql?: string | null }[] | null;
  result?: QueryResult;
}

/** Test glue until #33: ask once, poll to the terminal status, fetch the first query item. */
async function finalPoll(script: ScriptedResponse): Promise<Final> {
  const d = createGenieOneMcpDouble({ host: HOST, workspaceId: WSID, responses: [script] });
  const a = (await d.call('genie_ask', { question: 'q' })).structuredContent as {
    conversation_id: string;
    response_id: string;
  };
  const ref = { conversation_id: a.conversation_id, response_id: a.response_id };
  for (let i = 0; i < 10; i++) {
    const p = (await d.call('genie_poll_response', ref)).structuredContent as Final;
    if (p.status === 'completed' || p.status === 'incomplete' || p.status === 'failed') {
      const item = p.query_items?.[0];
      if (!item) return p;
      const r = await d.call('genie_get_query_result', { ...ref, item_id: item.item_id });
      return { ...p, result: r.structuredContent as QueryResult };
    }
  }
  throw new Error('no terminal status');
}

const asTable = (r: QueryResult): Table => ({
  columns: r.columns.map((c) => c.name),
  rows: r.rows.map((row) => row.map(String)),
});

const OUTCOMES = {
  completed: scenarios.normalAnswer,
  incomplete: scenarios.incompleteWithAnswer,
  incompleteNoAnswer: scenarios.incompleteNoAnswer,
  failed: scenarios.failed,
  truncated: () => scenarios.truncated(60),
  needsInput: scenarios.needsInput,
} as const;

describe('AT-09 show every response outcome', () => {
  it('[AT-09] [UX-11a] [UX-12] six outcomes keep 3 blurbs, newest on top, the rest in History', async () => {
    const stack = new BlurbStack();
    let t = 0;
    for (const [id, make] of Object.entries(OUTCOMES)) {
      const f = await finalPoll(make());
      stack.add({ id, text: f.final_answer ?? `(${f.status})`, completed: true }, (t += 1000));
    }
    const v = stack.view();
    expect(v.blurbs.map((b) => b.id)).toEqual(['needsInput', 'truncated', 'failed']);
    expect(v.historyCount).toBe(3);
    // UX-12: 60 s with no hover collapses the completed blurbs into History.
    stack.tick(t + 60_000);
    expect(stack.view().blurbs).toEqual([]);
    expect(stack.view().historyCount).toBe(6);
  });

  it('[AT-09] [UX-11b] [UX-11c] a 3-part split takes one slot and a pinned blurb stays', async () => {
    const stack = new BlurbStack();
    const f = await finalPoll(OUTCOMES.completed());
    stack.add({ id: 'pinned', text: f.final_answer ?? '', completed: true }, 1);
    stack.pin('pinned');
    for (const part of ['p1', 'p2', 'p3'])
      stack.add({ id: part, text: part, splitGroupId: 'split-1', completed: true }, 2);
    for (const id of ['n1', 'n2', 'n3']) stack.add({ id, text: id, completed: true }, 3);
    const v = stack.view();
    expect(v.blurbs.map((b) => b.id)).toContain('pinned');
    expect(v.blurbs).toHaveLength(3);
    const s = new BlurbStack();
    for (const part of ['p1', 'p2', 'p3'])
      s.add({ id: part, text: part, splitGroupId: 'split-1', completed: true }, 2);
    expect(s.view().blurbs).toHaveLength(1);
    expect(s.view().blurbs[0]?.parts).toHaveLength(3);
  });

  it('[AT-09] [UX-18] the expanded card of a completed answer shows every section in order', async () => {
    const f = await finalPoll(OUTCOMES.completed());
    if (!f.result || !f.final_answer || !f.deep_link) throw new Error('scenario changed');
    const ids = buildSections({
      answerMd: f.final_answer,
      keyNumbers: ['$1.2M'],
      sql: f.query_items?.[0]?.sql ?? '',
      table: asTable(f.result),
      sentToGenie: 'What was revenue last quarter?',
      canResend: true,
      deepLink: f.deep_link,
      dataFlowText: 'Sent to Genie One in your workspace.',
    }).map((s) => s.id);
    expect(ids).toEqual([
      'answer',
      'keyNumbers',
      'sql',
      'tablePreview',
      'sentToGenie',
      'resend',
      'copy',
      'footer',
      'dataFlow',
    ]);
  });

  it('[AT-09] [G-24b] [UX-18] a truncated 60-row result shows the first 50 rows only', async () => {
    const f = await finalPoll(OUTCOMES.truncated());
    if (!f.result) throw new Error('scenario changed');
    expect(f.result.truncated).toBe(true);
    const preview = buildSections({ table: asTable(f.result), dataFlowText: 'x' }).find(
      (s) => s.id === 'tablePreview',
    );
    if (preview?.id !== 'tablePreview') throw new Error('no table preview');
    expect(preview.rows).toHaveLength(PREVIEW_ROWS);
    expect(preview.rows[PREVIEW_ROWS - 1]?.[0]).toBe('acct-50');
    expect(preview.rows.some((r) => r[0] === 'acct-51')).toBe(false);
    expect(preview.moreRows).toBe(10);
  });

  it('[AT-09] [G-38b] a suggestion chip sends nothing until the user clicks it', () => {
    const row = new ChipRow();
    expect(row.offer({ id: 'c1', kind: 'ask_genie', label: 'Ask Genie' }, 0)).toBe(true);
    // An offer, a view and an expiry tick are not actions.
    row.view();
    row.tick(1000);
    expect(row.click('c1')).toEqual({ id: 'c1', kind: 'ask_genie' });
    expect(row.click('missing')).toBeUndefined();
  });

  it('[AT-09] [UX-01a] the mood shows success after a completed answer and error after a failure', () => {
    const at = 10_000;
    const src = (phase: RequestPhase): MoodSources => ({
      mog: { lifecycle: 'ready', since: 0 },
      capture: { recording: false, paused: false, othersTrack: 'off' },
      requests: [{ phase, at }],
    });
    expect(mapMood(MOG_A, src('completed'), at + 100).mood).toBe('success');
    expect(mapMood(MOG_A, src('failed'), at + 100).mood).toBe('error');
    expect(mapMood(MOG_A, src('working'), at + 100).mood).toBe('processing');
    // The success and error windows end (2.5 s and 3 s).
    expect(mapMood(MOG_A, src('completed'), at + 2_600).mood).toBe('idle');
    expect(mapMood(MOG_A, src('failed'), at + 3_100).mood).toBe('idle');
  });

  it('[AT-09] [UX-26a] [UX-26b] [UX-26c] answers with the card hidden give one merged notification', async () => {
    const clock = createSimClock();
    let timers: { at: number; fn: () => void }[] = [];
    const schedule: Scheduler = (at, fn) => {
      const t = { at, fn };
      timers.push(t);
      return () => void (timers = timers.filter((x) => x !== t));
    };
    const sent: OsNotification[] = [];
    const svc = createNotificationService({
      clock,
      schedule,
      host: {
        cardVisible: async () => false,
        focusAllows: async () => true,
        notify: async (n) => void sent.push(n),
      },
    });
    const flush = async (ms: number) => {
      clock.advance(ms);
      const due = timers.filter((t) => t.at <= clock.now());
      timers = timers.filter((t) => t.at > clock.now());
      due.forEach((t) => t.fn());
      for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
    };
    const f = await finalPoll(OUTCOMES.completed());
    for (let i = 0; i < 3; i++)
      svc.report({ kind: 'answer', mogId: MOG_A, text: f.final_answer ?? '' });
    svc.report({ kind: 'answer', mogId: MOG_B, text: 'Done.' });
    svc.report({ kind: 'chip', mogId: MOG_B, text: 'Ask Genie' });
    await flush(5_000);
    await flush(10_000);
    expect(sent).toHaveLength(1);
    expect(sent[0]?.mogIds).toEqual([MOG_A, MOG_B]);
    svc.dispose();
  });

  it.skip('[AT-09] [G-21a] [G-21b] [G-22] [G-23a] [G-23b] each outcome gives one normalised terminal event TODO(#33)', () => {
    // When #33 lands: run every OUTCOMES script through `GenieBridge.submit`; each gives exactly
    // one terminal AskEvent; incomplete with an answer is answer{incomplete:true} ("Partial
    // answer"), incomplete without one is failed; needsInput is needs_input{kind:'clarify'}.
    expect.unreachable('#33: the event normaliser is not built');
  });

  it.skip('[AT-09] [UX-13] [UX-14a] [UX-14b] [UX-15] [UX-17] blurb lines and verbatim key numbers TODO(#86)', () => {
    // When #86 lands: the completed blurb bolds "$1.2M" verbatim from the answer; a summary with
    // a new number falls back to the answer text; line 3 is "<profile> · Genie One · Open in
    // Databricks"; every string comes from Appendix E.
    expect.unreachable('#86: the blurb templates are not built');
  });

  it.skip('[AT-09] [UX-16a] [UX-16b] the progress line never shows banned words and updates every 2 s or less often TODO(#88)', () => {
    // When #88 lands: feed progress_steps that contain MCP, backend, polling, poll and thread;
    // the line shows none of them, and the update log has gaps of 2 s or more on the sim clock.
    expect.unreachable('#88: the progress line filter is not built');
  });

  it.skip('[AT-09] [G-12a] [G-12b] [G-12c] soft notice at 4 min, give-up at 10 min, exchange abandoned TODO(#149)', () => {
    expect.unreachable('#149: the long-ask rules are not built');
  });

  it.skip('[AT-09] [G-24a] [G-24c] the table loads only on expand; truncated shows "Full result in Databricks" TODO(#150)', () => {
    expect.unreachable('#150: the on-demand table preview is not built');
  });

  it.skip('[AT-09] [G-38a] each terminal answer is saved as one genie_exchange row TODO(#40)', () => {
    expect.unreachable('#40: the exchange store is not built');
  });
});
