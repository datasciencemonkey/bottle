/**
 * Red skeleton of the brain audit client (#140, ADR 0059).
 */
import type { MogId } from '../shared/interfaces';
import type { CommandArgs, CommandReturns } from '../shared/ipc';

export const AGENT_TOOLS = [] as const;
export type AgentTool =
  | 'transcript.search'
  | 'memory.recall'
  | 'memory.write'
  | 'show_blurb'
  | 'set_mood'
  | 'propose_page'
  | 'ask_user'
  | 'genie_one';
export type AuditKind = 'ask' | 'context_send' | 'page_call' | 'agent_action' | 'sync';

export interface AuditEvent {
  kind: AuditKind;
  tool?: AgentTool;
  mogId: MogId;
  opId: string;
  sizes?: Partial<Record<string, number>>;
  hashes?: Partial<Record<string, string>>;
}

export interface AuditRecord {
  kind: string;
  mogId: MogId;
  opId: string;
  sizes: Record<string, number>;
  hashes: Record<string, string>;
}

export class AuditRejectedError extends Error {
  readonly code: string = '';
}

export function textSizes(_text: string): { chars: number; bytes: number } {
  return { chars: -1, bytes: -1 };
}

export async function sha256Hex(_text: string): Promise<string> {
  return '';
}

export function buildAuditRecord(_e: AuditEvent): AuditRecord {
  throw new Error('not built');
}

export interface AuditLogDeps {
  invoke?: (
    name: 'audit_append',
    args: CommandArgs['audit_append'],
  ) => Promise<CommandReturns['audit_append']>;
}

export function createAuditLog(_deps: AuditLogDeps = {}): {
  record(e: AuditEvent): Promise<AuditRecord>;
} {
  return {
    record: () => Promise.reject(new Error('not built')),
  };
}
