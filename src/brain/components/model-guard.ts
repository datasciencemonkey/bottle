/**
 * Small-model calls never carry tools (#117; RTR-20a, ADR 0056).
 *
 * The router, cleanup and writer calls (and judge, embed and merge) go through `model_invoke`.
 * None of them may use tools or function calls. `toolFreeInvoker` refuses a request with a tool
 * field before it leaves the brain; the R12 gateway in Rust refuses it a second time
 * (`mog_core::model_request_guard`). ASR goes through `stt_transcribe`, which has no request body.
 */
import type { CommandArgs, CommandReturns, JsonObject } from '../../shared/ipc';

/** The chat-completions fields that turn on tools or function calls. */
export const TOOL_FIELDS = [
  'tools',
  'tool_choice',
  'functions',
  'function_call',
  'parallel_tool_calls',
] as const;

export type ToolField = (typeof TOOL_FIELDS)[number];

export type ModelInvoker = (
  args: CommandArgs['model_invoke'],
) => Promise<CommandReturns['model_invoke']>;

/** The tool fields at the top level of a chat-completions body, where the API reads them. */
export function findToolFields(request: JsonObject): ToolField[] {
  return TOOL_FIELDS.filter((f) => Object.hasOwn(request, f));
}

/** A refused small-model request. The message names the fields only, never request content. */
export class ToolFieldError extends Error {
  constructor(
    readonly role: CommandArgs['model_invoke']['role'],
    readonly fields: ToolField[],
  ) {
    super(`model_invoke role ${role} refused: tool fields ${fields.join(', ')} are not allowed`);
    this.name = 'ToolFieldError';
  }
}

/** Wraps an invoker so that every request with a tool field rejects and never reaches `inner`. */
export function toolFreeInvoker(inner: ModelInvoker): ModelInvoker {
  return async (args) => {
    const fields = findToolFields(args.request);
    if (fields.length) throw new ToolFieldError(args.role, fields);
    return inner(args);
  };
}
