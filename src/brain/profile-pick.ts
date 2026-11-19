/**
 * The profile picker of an Ask Genie press with no home profile (#163, #557, ADR 0092, ADR 0096).
 *
 * - #557 (user decision): every utterance is dictation first, so the spoken text is already
 *   pasted or on the Copy card. A press on Ask Genie with no home profile opens this picker.
 * - W-07d (picker part): it lists the `profiles_list` profiles that Bottle supports as chips
 *   (Appendix E "Choose profile"). The user clicks one. Bottle never picks, not even when only
 *   one profile is listed. Only the name is saved, as `mog.binding.home`.
 * - B1: the picker never sends. After the user saves a pick, `then` runs once: the ask gate
 *   sends the pressed text (the press was the confirmation).
 * - Every chip acts only on a click (UX-20c).
 */
import type { MogId, Profile, ProfileName } from '../shared/interfaces';
import type { ChipAction } from './card-chips';
import type { CardStack } from './card-stack';

export interface ProfilePickDeps {
  stackFor(mogId: MogId): Pick<CardStack, 'add' | 'offerChip' | 'removeChip'>;
  /** `window_card_show`. */
  showCard(mogId: MogId): Promise<unknown>;
  /** `profiles_list` (T7). */
  listProfiles(): Promise<Profile[]>;
  /** Save the user's pick as the home profile (T1 `edit`, `config_write`). */
  saveHome(mogId: MogId, profile: ProfileName): Promise<unknown>;
}

export interface ProfilePick {
  /** Show the picker on the card of `mogId`. `then` runs once after the user saves a pick. */
  choose(mogId: MogId, then: () => void): Promise<void>;
  /** A click on one of the picker's chips. Other chips are not the picker's and do nothing. */
  chip(mogId: MogId, action: ChipAction): Promise<void>;
}

export function createProfilePick(deps: ProfilePickDeps): ProfilePick {
  /** The click handler of each profile chip, by chip ID. */
  const actions = new Map<string, () => Promise<void>>();
  /** The MOGs whose picker is open. */
  const open = new Set<MogId>();
  let n = 0;
  const note = (mogId: MogId, text: string) =>
    deps.stackFor(mogId).add({ id: `pick-${mogId}-note-${++n}`, text, completed: true });

  return {
    async choose(mogId, then) {
      // A second press while the list is open does not offer the profiles again.
      if (open.has(mogId)) return;
      open.add(mogId);
      try {
        await deps.showCard(mogId);
      } catch {
        // The card stack keeps the list for the next show.
      }
      let supported: Profile[];
      try {
        supported = (await deps.listProfiles()).filter((p) => p.supported);
      } catch {
        supported = [];
      }
      if (supported.length === 0) {
        open.delete(mogId);
        note(
          mogId,
          'Bottle cannot list your Databricks CLI profiles. Sign in with `databricks auth login`, then set the home profile of this MOG in the hub.',
        );
        return;
      }
      note(mogId, 'Which workspace? Click the home profile for this MOG.');
      const stack = deps.stackFor(mogId);
      const idOf = (p: Profile) => `pick-${mogId}-use-${p.name}`;
      for (const p of supported) {
        actions.set(idOf(p), async () => {
          try {
            await deps.saveHome(mogId, p.name);
          } catch {
            note(mogId, 'Bottle could not save the home profile. Try again.');
            return;
          }
          for (const q of supported) {
            stack.removeChip(idOf(q));
            actions.delete(idOf(q));
          }
          open.delete(mogId);
          then();
        });
        stack.offerChip({
          id: idOf(p),
          kind: 'choose_profile',
          label: p.name,
          owner: `pick-${mogId}`,
        });
      }
    },
    async chip(_mogId, action) {
      await actions.get(action.id)?.();
    },
  };
}
