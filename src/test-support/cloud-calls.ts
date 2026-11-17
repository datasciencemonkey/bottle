// Which brain IPC calls leave the device (#231, AT-04 "no cloud request", AT-43 "no agent turn
// and no Genie call"). Brain tests mock `@tauri-apps/api/core` with `src/brain/test-core.ts`, so
// `core.invoke.mock.calls` is the list of `[command, { args }]` the brain sent. The core is the only
// network path (ARC-4), so a brain call that is classed `local` sends nothing off the device.
//
// The class map is typed over every 4.5 command: a new command fails the typecheck until someone
// classes it. The helpers return command names only, never argument values (SEC rules).
import { COMMANDS } from '../shared/ipc';

export type CallClass = 'local' | 'cloud' | 'cloud_unless_local_stt';

export const CALL_CLASS = {
  cli_status: 'local',
  cli_install: 'cloud', // downloads the official CLI
  profiles_list: 'local',
  profile_status: 'cloud', // the CLI may refresh the token over the network
  profile_login: 'cloud',
  profile_select: 'local',
  capture_start: 'local',
  capture_stop: 'local',
  capture_pause: 'local',
  capture_resume: 'local',
  capture_status: 'local',
  capture_stop_all: 'local',
  capture_subscribe: 'local',
  session_attest: 'local',
  consent_notice_record: 'local',
  consent_records_list: 'local',
  stt_transcribe: 'cloud_unless_local_stt',
  insert_text: 'local',
  agent_start: 'cloud', // the harness process can reach the network
  agent_stop: 'local',
  agent_send: 'cloud', // the harness model call goes through the AI Gateway
  agent_tool_result: 'cloud', // resumes the agent turn: a model call through the AI Gateway
  model_invoke: 'cloud',
  mcp_call: 'cloud',
  dbx_http: 'cloud',
  mem_local_exec: 'local',
  mem_remote_exec: 'cloud',
  mem_remote_status: 'cloud', // v1 Lakebase status
  config_list: 'local',
  config_read: 'local',
  config_write: 'local',
  config_delete: 'local',
  policy_get: 'local',
  hotkeys_set: 'local',
  permission_request: 'local',
  app_setting_read: 'local',
  app_setting_write: 'local',
  mog_set_active: 'local',
  window_mog_open: 'local',
  window_mog_close: 'local',
  window_card_show: 'local',
  window_card_hide: 'local',
  view_emit: 'local',
  audit_append: 'local',
  telemetry_span: 'cloud', // exported once an admin sets OtlpTable (4.5, SEC-16a)
  brain_subscribe: 'local',
} as const satisfies Record<keyof typeof COMMANDS, CallClass>;

/** The command arguments of an invoke log entry: `call()` sends them as `{ args }` (ADR 0003). */
export function argsOf(payload: unknown): unknown {
  if (payload !== null && typeof payload === 'object' && 'args' in payload) {
    return (payload as { args: unknown }).args;
  }
  return payload;
}

/**
 * True when the call would leave the device. `args` is the command arguments or the `{ args }`
 * payload that `invoke` gets. Unknown commands and shapes count as cloud.
 */
export function isCloudCall(cmd: string, args: unknown): boolean {
  const cls = (CALL_CLASS as Record<string, CallClass | undefined>)[cmd];
  if (cls === undefined || cls === 'cloud') return true;
  if (cls === 'local') return false;
  const provider = (argsOf(args) as { provider?: unknown } | null | undefined)?.provider;
  return provider !== 'local_parakeet';
}

type Log = readonly (readonly unknown[])[];

/** The names of the calls in `log` that would leave the device, in order. */
export function cloudCalls(log: Log): string[] {
  return log.filter(([cmd, args]) => isCloudCall(String(cmd), args)).map(([cmd]) => String(cmd));
}

/** The names of the calls in `log` to any of `commands`, in order. */
export function callsTo(log: Log, commands: readonly string[]): string[] {
  return log.map(([cmd]) => String(cmd)).filter((cmd) => commands.includes(cmd));
}
