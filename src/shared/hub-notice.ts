/**
 * The versioned first-run notice "Before you start" (9.3, MB-08a, MB-08b, ADR 0044).
 *
 * The brain (T12 ConsentService) and the hub view both read this file. It holds static data and
 * shape checks only, no state. It is not a contract file.
 * TODO(#393): move `HUB_NOTICE_EVENT`, `HubNotice` and the hub notice actions into
 * `src/shared/ipc.ts` (`EVENTS['hub.notice']` and hub-level view_emit kinds).
 */

/**
 * The current notice version. Increase it by 1 for each change of the notice text: each user then
 * sees the notice again, and capture stays blocked until the user accepts it (MB-08b).
 */
export const NOTICE_VERSION = 1;

/** The brain-to-view event that carries the notice state to the hub. TODO(#393): `EVENTS`. */
export const HUB_NOTICE_EVENT = 'hub:notice';

/**
 * The `mogId` of a hub-level `view_emit` (the notice is not about one MOG). It cannot be a MOG ID,
 * because MOG IDs are UUIDs. TODO(#393): a hub-level view input with no `mogId`.
 */
export const HUB_SCOPE = 'hub';

/** What the hub shows: the current version, and whether the user accepted it. */
export interface HubNotice {
  version: number;
  accepted: boolean;
}

/** The hub notice actions, sent as `view_emit` kind `click` with mogId `HUB_SCOPE`. */
export type NoticeAction =
  { action: 'notice.ready' } | { action: 'notice.accept'; noticeVersion: number };

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const isVersion = (v: unknown): v is number =>
  typeof v === 'number' && Number.isInteger(v) && v >= 1;

/** Shape check of a `hub:notice` payload. */
export function asHubNotice(p: unknown): HubNotice | undefined {
  if (!isObj(p) || !isVersion(p['version']) || typeof p['accepted'] !== 'boolean') return undefined;
  return { version: p['version'], accepted: p['accepted'] };
}

/** Shape check of a hub notice action payload. */
export function asNoticeAction(p: unknown): NoticeAction | undefined {
  if (!isObj(p)) return undefined;
  if (p['action'] === 'notice.ready') return { action: 'notice.ready' };
  if (p['action'] === 'notice.accept' && isVersion(p['noticeVersion']))
    return { action: 'notice.accept', noticeVersion: p['noticeVersion'] };
  return undefined;
}

/**
 * The notice copy for `NOTICE_VERSION`. It covers each item of 9.3 step 2, in plain text.
 * TODO(#394): DRAFT. This copy follows the requirement wording. It is not approved legal text.
 * Legal or admin review must approve or replace it, and then increase `NOTICE_VERSION`.
 */
export const LEGAL_NOTICE = {
  title: 'Before you start',
  draft: true,
  paragraphs: [
    {
      id: 'records',
      text: 'Bottle records your voice only when you act: you hold the Talk key, start hands-free dictation, or start a meeting session.',
    },
    {
      id: 'mic',
      text: 'The microphone stays closed unless you act. Bottle shows when it listens.',
    },
    {
      id: 'where',
      text: 'Bottle turns speech into text on this computer by default. Text goes to your Databricks workspace only when you send a question, a note or a Page.',
    },
    {
      id: 'retention',
      text: 'Talk key audio stays in memory and is deleted after it becomes text. Meeting audio is deleted 5 minutes or less after the transcript is saved.',
    },
    {
      id: 'retention-text',
      text: 'Transcripts, answers and notes stay on this computer until you use Wipe in Settings. Consent records are kept for 13 months.',
    },
    {
      id: 'consent',
      text: 'Many places require the consent of all persons before a conversation is recorded. Get consent from each person before you record other people.',
    },
    {
      id: 'biometrics',
      text: 'Bottle makes no voiceprints. It does no speaker identification and no emotion recognition.',
    },
  ],
  checkboxLabel: 'I read this notice. I will get consent before I record other people.',
  continueLabel: 'Continue',
} as const;
