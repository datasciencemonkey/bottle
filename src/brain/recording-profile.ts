/**
 * The one workspace of a call recording (ADR 0103). Every cloud-bound recording command, and the
 * record key (⌥R), carries this profile, so they all reach the same workspace.
 *
 * Bottle never picks between workspaces: it uses a home profile only when every MOG that has a
 * home has the same one. No home at all is `no_profile`; two or more different homes is
 * `profiles_differ`. Both reject with `{ error }`, like a refused command.
 */
import type { MogId, ProfileName } from '../shared/interfaces';

export interface RecordingProfileDeps {
  /** The defined MOGs. */
  mogs(): Promise<MogId[]>;
  /** The usable home profile of a MOG, or undefined. */
  home(mogId: MogId): Promise<ProfileName | undefined>;
}

export async function recordingProfile(deps: RecordingProfileDeps): Promise<ProfileName> {
  let homes: (ProfileName | undefined)[];
  try {
    const ids = await deps.mogs();
    homes = await Promise.all(ids.map((id) => deps.home(id)));
  } catch {
    throw { error: 'no_profile' };
  }
  const distinct = new Set(homes.filter((h): h is ProfileName => h !== undefined && h !== ''));
  if (distinct.size === 0) throw { error: 'no_profile' };
  if (distinct.size > 1) throw { error: 'profiles_differ' };
  return [...distinct][0] as ProfileName;
}
