/** The plain words for an orchestrator model that is not ready (ADR 0122). */
import type { OrchestratorState } from '../../shared/ipc';

/** Why the orchestrator model is not ready, in one sentence. `ready` has no reason. */
export function notReadyReason(
  state: Exclude<OrchestratorState, 'ready'>,
  homeName: string | null,
): string {
  const home = homeName ?? 'your home workspace';
  switch (state) {
    case 'no_home':
      return 'Choose a home workspace first.';
    case 'signed_out':
      return `Your sign-in to ${home} expired.`;
    case 'no_model':
      return `This model is not available in ${home}. Pick another.`;
    case 'unreachable':
      return 'The orchestrator model is not reachable.';
  }
}

/** The tail of the reason: where a question goes instead. */
export const DIRECT_TAIL = 'Questions go to your home workspace only.';
