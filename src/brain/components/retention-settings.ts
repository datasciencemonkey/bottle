/**
 * The Settings retention rows of section 9.2, with the admin lock (#124; SEC-15a, SEC-15c, MB-15,
 * U9, COL-101). See ADR 0087.
 *
 * - SEC-15c: if an admin policy and a user setting differ, the admin policy applies. MB-15: the
 *   panel shows each MDM value with a lock and "Set by your admin", and the user cannot change it.
 *   {@link adminWins} is that rule.
 * - U9: user content (dictation history, transcripts, Genie answers, memory items) is kept until
 *   the user wipes. Appendix C leaves its keys unset (U9). An admin value for them does not turn
 *   on an automatic delete in the MVP: COL-101 keeps an MDM Wipe policy for v1.
 * - Content-free security data keeps its expiry. `AudioRetentionDays` (0-30, default 0) is the
 *   one Appendix C key here. Appendix C names no key for the log, action log and consent
 *   periods yet, so they show the 9.2 defaults. TODO(#186): PolicyStore gives those keys.
 */
import { call } from '../../shared/ipc';
import type { RetentionRow, WipeSettingsModel } from '../../shared/wipe-view';

/** SEC-15c: the admin value wins; it is locked whenever the admin sets it (MB-15). */
export function adminWins<T>(user: T, admin: T | undefined): { value: T; locked: boolean } {
  return admin === undefined ? { value: user, locked: false } : { value: admin, locked: true };
}

const KEPT = 'Kept until you wipe';
const AUDIO_DEFAULT = 'Deleted within 5 minutes after the session is saved';

const days = (n: number) => `${n} ${n === 1 ? 'day' : 'days'}`;

/** A whole number in `min..=max`, or undefined (not a valid policy value). */
function intIn(v: unknown, min: number, max: number): number | undefined {
  return typeof v === 'number' && Number.isInteger(v) && v >= min && v <= max ? v : undefined;
}

/** The 9.2 rows with the admin policy applied. */
export function retentionSettings(policy: Readonly<Record<string, unknown>>): WipeSettingsModel {
  const kept = (id: string, label: string): RetentionRow => ({
    id,
    label,
    value: KEPT,
    locked: false,
  });
  const audio = adminWins(0, intIn(policy['AudioRetentionDays'], 0, 30));
  return {
    rows: [
      kept('dictation_history', 'Dictation history'),
      kept('transcripts', 'Transcripts'),
      kept('genie_answers', 'Genie answers'),
      kept('memory_items', 'Memory notes'),
      {
        id: 'audio_journal',
        label: 'Session audio journal',
        value: audio.value === 0 ? AUDIO_DEFAULT : days(audio.value),
        locked: audio.locked,
      },
      { id: 'local_logs', label: 'Local logs', value: '7 days', locked: false },
      { id: 'action_log', label: 'Agent action log', value: '30 days', locked: false },
      { id: 'consent_records', label: 'Consent records', value: '13 months', locked: false },
    ],
  };
}

/**
 * The admin policy from R13 PolicyStore (`policy_get`). TODO(#186): the core answers
 * `not_implemented` until PolicyStore reads the MDM keys, so this answers no policy, and no row
 * is locked.
 */
export async function readAdminPolicy(): Promise<Record<string, unknown>> {
  try {
    const p = await call('policy_get', {});
    return typeof p === 'object' && p !== null && !Array.isArray(p)
      ? { ...(p as Record<string, unknown>) }
      : {};
  } catch {
    return {};
  }
}
