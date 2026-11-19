/**
 * The sign-in state of a Databricks CLI profile, derived from Genie outcomes this session
 * (#560, #486). `profile_status` is not routed host-side yet (#486; its contract PR is pending
 * approval), so the Workspaces area cannot ask the host whether a profile's token is valid. This
 * module is the stand-in: it watches the end of each Ask Genie request (`genie-direct.ts`
 * `onEnd`, the same `GenieEnd` that `genie-history.ts` saves) and remembers, per profile, whether
 * the last one this session succeeded or was refused for an auth reason.
 *
 * It never calls the CLI or a token route itself, and it holds no state across app restarts.
 */
import type { ProfileName } from '../shared/interfaces';
import type { GenieEnd } from './genie-history';

/** See `src/shared/hub-workspaces.ts`. */
export type SignInState = 'signed_in' | 'needs_sign_in' | 'unknown';

/**
 * The GL-5 codes (genie-errors.ts, ADR 0072) whose fixed action is `sign_in`: the profile's
 * OAuth token needs a fresh `databricks auth login`, or its auth type cannot sign in that way.
 */
const SIGN_IN_NEEDED_CODES = new Set(['AUTH_EXPIRED', 'UNSUPPORTED_AUTH']);

export interface SignInTracker {
  /** Records the outcome of one ended Ask Genie request, keyed by its home profile. */
  record(end: Pick<GenieEnd, 'profile' | 'end'>): void;
  /**
   * The sign-in state of `profile` from the last recorded outcome this session, or `unknown`
   * with none yet. TODO(#486): switch to `profile_status` once the host routes it.
   */
  signInState(profile: ProfileName): SignInState;
}

export function createSignInTracker(): SignInTracker {
  const state = new Map<ProfileName, SignInState>();
  return {
    record(end) {
      if (end.end.t === 'answer' || end.end.t === 'needs_input') {
        state.set(end.profile, 'signed_in');
      } else if (end.end.t === 'failed' && SIGN_IN_NEEDED_CODES.has(end.end.code)) {
        state.set(end.profile, 'needs_sign_in');
      }
      // Every other outcome (rate limited, offline, cancelled, ...) says nothing about sign-in:
      // the last known state for this profile stays.
    },
    signInState(profile) {
      return state.get(profile) ?? 'unknown';
    },
  };
}
