/**
 * The texts and the command of a workspace sign-in, shared by the brain
 * (`src/brain/workspaces.ts`) and the hub (Settings → Workspace). The view reads the notice by
 * its text: the notice has no code field (a change of `hub-workspaces.ts` would be needed).
 */

/** The notice while the browser sign-in runs. The hub shows Cancel while it is up. */
export const SIGN_IN_WAITING = 'Waiting for you to sign in in your browser…';

/** The notice for `profile_invalid`: the profile cannot be signed in again by Bottle. */
export const SIGN_IN_NOT_BROWSER =
  'Bottle cannot sign this workspace in again. Check the profile in your Databricks CLI config, then reopen Settings.';

/** The CLI sign-in of a profile, with a shell quote only when the name needs one. */
export const signInCommand = (profile: string): string =>
  `databricks auth login --profile ${
    /^[\w.-]+$/.test(profile) ? profile : `'${profile.replace(/'/g, `'\\''`)}'`
  }`;
