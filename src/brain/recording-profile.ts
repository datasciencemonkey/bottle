/**
 * The one workspace of a call recording (ADR 0103, ADR 0117). Every cloud-bound recording
 * command, and the record key (⌥R), carries this profile, so they all reach the same workspace.
 *
 * It is the home profile of the one MOG. Bottle never picks a home: with no home it is
 * `no_profile`. It rejects with `{ error }`, like a refused command.
 */
import type { ProfileName } from '../shared/interfaces';

export interface RecordingProfileDeps {
  /** The home profile (`WorkspaceList.home`), or undefined when the user has not picked one. */
  home(): Promise<ProfileName | undefined>;
}

export async function recordingProfile(deps: RecordingProfileDeps): Promise<ProfileName> {
  let home: ProfileName | undefined;
  try {
    home = await deps.home();
  } catch {
    throw { error: 'no_profile' };
  }
  if (home === undefined || home === '') throw { error: 'no_profile' };
  return home;
}
