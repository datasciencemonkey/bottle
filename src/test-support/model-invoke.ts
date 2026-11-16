// `model_invoke` test double for the router step (#57). A scripted, in-memory stand-in for the
// AI Gateway chat-completions endpoint that `model_invoke` (role `router`) reaches in Rust.
// It never calls a live workspace and never picks a Databricks CLI profile: the caller passes one.
// TODO(#56): SP-13 records real router responses on small models; add them here as fixtures.

import type { CommandArgs, CommandReturns } from '../shared/ipc';
import type { RouterDecision } from '../shared/interfaces';

export type ModelInvokeArgs = CommandArgs['model_invoke'];
export type ModelInvokeResult = CommandReturns['model_invoke'];

/** One scripted reply of the double. */
export type RouterReply =
  | { decision: Partial<RouterDecision> & Record<string, unknown> } // valid chat completion + JSON
  | { content: string } // chat completion whose message content is this raw string
  | { json: unknown } // the raw `model_invoke` result `json`
  | { error: unknown }; // `model_invoke` rejects (transport or gateway failure)

/** A full, valid router decision. `over` replaces fields. */
export function routerDecision(
  over: Partial<RouterDecision> & Record<string, unknown> = {},
): RouterDecision {
  return {
    intent: 'question',
    cleaned_text: 'What was revenue last quarter?',
    genie_question: 'What was revenue last quarter?',
    target_profile_hint: '',
    confidence: 0.9,
    follow_up: false,
    topic_label: 'revenue',
    part_questions: [],
    part_profile_hints: [],
    command: 'none',
    command_arg: '',
    context_note: '',
    injection_suspected: false,
    directed_mog_id: '',
    ...over,
  } as RouterDecision;
}

/** The chat-completions body that the AI Gateway returns for one message `content`. */
export function chatCompletion(content: string): ModelInvokeResult {
  return {
    json: {
      id: 'chatcmpl-double',
      object: 'chat.completion',
      choices: [{ index: 0, finish_reason: 'stop', message: { role: 'assistant', content } }],
    },
  };
}

export interface RouterModelDouble {
  readonly invoke: (args: ModelInvokeArgs) => Promise<ModelInvokeResult>;
  /** Every request the router step sent, in order. */
  readonly calls: ModelInvokeArgs[];
}

/**
 * A router model double. `script` is a list of replies used in call order (the last reply
 * repeats), or a function of the request and the 0-based call index.
 */
export function createRouterModelDouble(
  script: RouterReply[] | ((args: ModelInvokeArgs, n: number) => RouterReply),
): RouterModelDouble {
  const calls: ModelInvokeArgs[] = [];
  const pick = (args: ModelInvokeArgs, n: number): RouterReply => {
    if (typeof script === 'function') return script(args, n);
    const r = script[Math.min(n, script.length - 1)];
    if (!r) throw new Error('router model double has an empty script');
    return r;
  };
  return {
    calls,
    async invoke(args) {
      const n = calls.length;
      calls.push(structuredClone(args));
      const r = pick(args, n);
      if ('error' in r) throw r.error;
      if ('json' in r) return { json: r.json };
      if ('content' in r) return chatCompletion(r.content);
      return chatCompletion(JSON.stringify(routerDecision(r.decision)));
    },
  };
}
