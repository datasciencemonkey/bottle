import { clearUserWords } from '../shared/dictation-vocab';
import type { WipeDialogDeps } from './wipe-wiring';

/**
 * "Wipe everything" also clears the user's Dictation words, which can hold names. A wipe that
 * throws leaves the list as it is. An older-than wipe keeps it.
 */
export function clearingDictationWords(
  flow: WipeDialogDeps['flow'],
  clear: () => void = clearUserWords,
): WipeDialogDeps['flow'] {
  return {
    dialog: (scope, alsoGenie) => flow.dialog(scope, alsoGenie),
    async confirm(scope, alsoGenie) {
      const done = await flow.confirm(scope, alsoGenie);
      if (scope === 'everything') clear();
      return done;
    },
  };
}
