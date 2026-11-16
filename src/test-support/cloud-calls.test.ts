import { COMMANDS } from '../shared/ipc';
import { CALL_CLASS, callsTo, cloudCalls, isCloudCall } from './cloud-calls';

const tags = { app: 'bottle', mog: 'm1', op: 'asr' } as const;

describe('cloud-call classifier (AT-04, AT-43 harness)', () => {
  it('[AT-22] every 4.5 command has a class', () => {
    expect(Object.keys(CALL_CLASS).sort()).toEqual(Object.keys(COMMANDS).sort());
  });

  it('[AT-22] Genie, model, HTTP, agent and login calls leave the device', () => {
    for (const cmd of [
      'mcp_call',
      'model_invoke',
      'dbx_http',
      'agent_send',
      'mem_remote_exec',
      'profile_login',
      'cli_install',
    ]) {
      expect(isCloudCall(cmd, {}), cmd).toBe(true);
    }
  });

  it('[AT-22] local STT, insertion, the local store and windows stay on the device', () => {
    for (const cmd of [
      'insert_text',
      'mem_local_exec',
      'capture_start',
      'config_write',
      'window_card_show',
      'audit_append',
    ]) {
      expect(isCloudCall(cmd, {}), cmd).toBe(false);
    }
    expect(
      isCloudCall('stt_transcribe', { segmentId: 's', provider: 'local_parakeet', tags }),
    ).toBe(false);
  });

  it('[AT-22] stt_transcribe is a cloud call for a Databricks provider or a malformed one', () => {
    expect(isCloudCall('stt_transcribe', { segmentId: 's', provider: 'dbx_gemini', tags })).toBe(
      true,
    );
    expect(isCloudCall('stt_transcribe', { segmentId: 's', provider: 'dbx_whisper', tags })).toBe(
      true,
    );
    // Unknown shapes count as cloud, so a bug cannot hide a request.
    expect(isCloudCall('stt_transcribe', undefined)).toBe(true);
    expect(isCloudCall('not_a_command', {})).toBe(true);
  });

  it('[AT-22] reads an invoke log and names only the commands', () => {
    const log: unknown[][] = [
      ['stt_transcribe', { segmentId: 's', provider: 'local_parakeet', tags }],
      ['insert_text', { mogId: 'm1', text: 'secret words' }],
      ['mcp_call', { tool: 'genie_ask' }],
      ['agent_send', { mogId: 'm1', turn: {} }],
    ];
    expect(cloudCalls(log)).toEqual(['mcp_call', 'agent_send']);
    expect(callsTo(log, ['agent_send', 'mcp_call'])).toEqual(['mcp_call', 'agent_send']);
    expect(callsTo(log, ['insert_text'])).toEqual(['insert_text']);
    // The result holds command names only, never the text of an argument.
    expect(JSON.stringify(cloudCalls(log))).not.toContain('secret');
  });
});
