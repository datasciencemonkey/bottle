import { clearUserWords } from '../shared/dictation-vocab';
import type { WipeDialogDeps } from './wipe-wiring';

/**
 * "Wipe everything" also clears the user's Dictation words, which can hold names. A wipe that
 * throws before the local wipe worked leaves the list as it is. An older-than wipe keeps it.
 */
export function clearingDictationWords(
  flow: WipeDialogDeps['flow'],
  clear: () => void = clearUserWords,
): WipeDialogDeps['flow'] {
  return {
    dialog: (scope, alsoGenie) => flow.dialog(scope, alsoGenie),
    async confirm(scope, alsoGenie, onLocalWiped) {
      // The words go when the local wipe worked, also if the remote deletes throw after it.
      let cleared = false;
      const clearOnce = () => {
        if (cleared || scope !== 'everything') return;
        cleared = true;
        clear();
      };
      const done = await flow.confirm(scope, alsoGenie, () => {
        try {
          clearOnce();
        } finally {
          onLocalWiped?.();
        }
      });
      clearOnce();
      return done;
    },
  };
}
