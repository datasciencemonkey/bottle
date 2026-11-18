/**
 * #124: the Settings retention rows of section 9.2 and the admin lock (SEC-15c, MB-15, SEC-15a,
 * U9; ADR 0087). Every policy value here is fake.
 */
import { core } from '../test-core';
import { adminWins, readAdminPolicy, retentionSettings } from './retention-settings';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);

beforeEach(() => core.reset());

const row = (id: string, policy: Record<string, unknown> = {}) => {
  const r = retentionSettings(policy).rows.find((x) => x.id === id);
  if (r === undefined) throw new Error(`no row ${id}`);
  return r;
};

describe('retention settings (#124)', () => {
  it('[SEC-15c] if the admin policy and the user setting differ, the admin policy applies and is locked', () => {
    expect(adminWins(true, false)).toEqual({ value: false, locked: true });
    expect(adminWins(0, 7)).toEqual({ value: 7, locked: true });
    // MB-15: each MDM value shows the lock, also when it equals the user setting.
    expect(adminWins(7, 7)).toEqual({ value: 7, locked: true });
    expect(adminWins(0, undefined)).toEqual({ value: 0, locked: false });
  });

  it('[SEC-15a] [U9] the rows follow the 9.2 table: user content is kept until you wipe', () => {
    const m = retentionSettings({});
    expect(m.rows.map((r) => r.id)).toEqual([
      'dictation_history',
      'transcripts',
      'genie_answers',
      'memory_items',
      'audio_journal',
      'local_logs',
      'action_log',
      'consent_records',
    ]);
    for (const id of ['dictation_history', 'transcripts', 'genie_answers', 'memory_items']) {
      expect(row(id).value).toBe('Kept until you wipe');
      expect(row(id).locked).toBe(false);
    }
    expect(row('audio_journal').value).toBe('Deleted within 5 minutes after the session is saved');
    expect(row('local_logs').value).toBe('7 days');
    expect(row('action_log').value).toBe('30 days');
    expect(row('consent_records').value).toBe('13 months');
  });

  it('[SEC-15c] [MB-15] an admin AudioRetentionDays shows its value with the lock', () => {
    const r = row('audio_journal', { AudioRetentionDays: 7 });
    expect(r).toEqual({
      id: 'audio_journal',
      label: 'Session audio journal',
      value: '7 days',
      locked: true,
    });
    expect(row('audio_journal', { AudioRetentionDays: 1 }).value).toBe('1 day');
    // A value outside the admin range 0-30 is not a valid policy: no lock, the default shows.
    for (const bad of [31, -1, 2.5, '7', null]) {
      expect(row('audio_journal', { AudioRetentionDays: bad }).locked).toBe(false);
    }
  });

  it('[U9] [COL-101] an admin retention key for user content does not turn on an automatic delete', () => {
    const m = retentionSettings({
      TranscriptRetentionDays: 30,
      DictationHistoryDays: 7,
      MemoryRetentionDays: 30,
      GenieExchangeRetentionDays: 30,
    });
    for (const id of ['dictation_history', 'transcripts', 'genie_answers', 'memory_items']) {
      expect(m.rows.find((r) => r.id === id)?.value).toBe('Kept until you wipe');
    }
  });

  it('[SEC-15c] TODO(#186): the policy comes from policy_get; no policy means no lock', async () => {
    core.invoke.mockResolvedValueOnce({ AudioRetentionDays: 3, Other: 'x' });
    expect(await readAdminPolicy()).toEqual({ AudioRetentionDays: 3, Other: 'x' });
    expect(core.invoke).toHaveBeenCalledWith('policy_get', { args: {} });
    core.invoke.mockRejectedValueOnce({ error: 'not_implemented' });
    expect(await readAdminPolicy()).toEqual({});
    core.invoke.mockResolvedValueOnce(['not', 'an', 'object']);
    expect(await readAdminPolicy()).toEqual({});
  });
});
