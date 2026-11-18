/**
 * The "Pick a profile to ask Genie" card of a question with no home profile (#163, ADR 0092).
 *
 * - RTR-15c, B1: the card never blocks and never sends. Nothing goes to Genie from it.
 * - The spoken text stays on the card, so a misread never loses it. The heuristic can read a
 *   dictation as a question ("What I mean is ..."), so the card offers [Insert as text], which
 *   sends the text down the dictation route (`insert_text`, or Copy when insertion is off), and
 *   [Copy]. TODO(#213): a click on Copy writes the clipboard; no clipboard command is on main.
 * - W-07d (picker part): [Pick a profile] lists the `profiles_list` profiles that Bottle
 *   supports as chips (Appendix E "Choose profile"). The user clicks one. Bottle never picks,
 *   not even when only one profile is listed. Only the name is saved, as `mog.binding.home`.
 * - Every chip acts only on a click (UX-20c).
 */
import type { MogId, Profile, ProfileName } from '../shared/interfaces';
import type { CommandReturns } from '../shared/ipc';
import type { ChipAction } from './card-chips';
import type { CardStack } from './card-stack';
import type { OwnedUtterance } from './request-owner';
import type { AskRoute } from './utterance-wiring';

export const PICK_TEXT = 'Pick a profile to ask Genie.';

export interface ProfilePickDeps {
  stackFor(mogId: MogId): Pick<CardStack, 'add' | 'offerChip' | 'removeChip' | 'removeChipsOf'>;
  /** `window_card_show`. */
  showCard(mogId: MogId): Promise<unknown>;
  /** `profiles_list` (T7). */
  listProfiles(): Promise<Profile[]>;
  /** Save the user's pick as the home profile (T1 `edit`, `config_write`). */
  saveHome(mogId: MogId, profile: ProfileName): Promise<unknown>;
  /** `app.dictation.insert` (Appendix F). */
  insertionOn(mogId: MogId): boolean;
  insertText(args: { mogId: MogId; text: string }): Promise<CommandReturns['insert_text']>;
}

export interface ProfilePick {
  /** Show the pick card for an ask route of a MOG with no home profile. */
  showNoProfile(r: AskRoute, owned: OwnedUtterance): Promise<void>;
  /** A click on one of this card's chips. Other chips are not this card's and do nothing. */
  chip(mogId: MogId, action: ChipAction): Promise<void>;
}

const firstNonEmpty = (...xs: string[]): string => xs.find((x) => x.trim() !== '') ?? '';

/** The spoken text of an ask route. */
const textOf = (r: AskRoute): string =>
  r.route === 'preview'
    ? r.question
    : firstNonEmpty(r.decision.cleaned_text, r.decision.genie_question);

export function createProfilePick(deps: ProfilePickDeps): ProfilePick {
  /** The click handler of each chip this card offered, by chip ID. */
  const actions = new Map<string, () => Promise<void>>();
  let n = 0;
  const note = (mogId: MogId, owner: string, text: string) =>
    deps.stackFor(mogId).add({ id: `${owner}-note-${++n}`, text, completed: true });

  const insert = async (mogId: MogId, owner: string, text: string) => {
    if (!deps.insertionOn(mogId)) {
      note(mogId, owner, 'Insertion is off. Use Copy.');
      return;
    }
    let result: string;
    try {
      result = (await deps.insertText({ mogId, text })).result;
    } catch {
      result = 'host_failed';
    }
    if (result === 'inserted') note(mogId, owner, 'Inserted as text.');
    else note(mogId, owner, 'Could not insert. The text stays here. Use Copy.');
  };

  const pickList = async (mogId: MogId, owner: string) => {
    // A second click on [Pick a profile] does not offer the profiles again.
    if ([...actions.keys()].some((id) => id.startsWith(`${owner}-use-`))) return;
    let supported: Profile[];
    try {
      supported = (await deps.listProfiles()).filter((p) => p.supported);
    } catch {
      supported = [];
    }
    if (supported.length === 0) {
      note(
        mogId,
        owner,
        'Bottle cannot list your Databricks CLI profiles. Sign in with `databricks auth login`, then set the home profile of this MOG in the hub.',
      );
      return;
    }
    note(mogId, owner, 'Which workspace? Click the home profile for this MOG.');
    const stack = deps.stackFor(mogId);
    for (const p of supported) {
      const id = `${owner}-use-${p.name}`;
      actions.set(id, async () => {
        try {
          await deps.saveHome(mogId, p.name);
        } catch {
          note(mogId, owner, 'Bottle could not save the home profile. Try again.');
          return;
        }
        for (const q of supported) stack.removeChip(`${owner}-use-${q.name}`);
        stack.removeChip(`${owner}-pick`);
        actions.delete(`${owner}-pick`);
        for (const q of supported) actions.delete(`${owner}-use-${q.name}`);
        note(mogId, owner, `${p.name} is the home profile of this MOG now. Ask again.`);
      });
      stack.offerChip({ id, kind: 'choose_profile', label: p.name, owner });
    }
  };

  return {
    async showNoProfile(r, owned) {
      const { mogId, utteranceId: owner } = owned;
      const text = textOf(r);
      try {
        await deps.showCard(mogId);
      } catch {
        // The card stack keeps the text for the next show. TODO(#507): `ready` sends it again.
      }
      const stack = deps.stackFor(mogId);
      stack.add({ id: owner, text: `${text}\n${PICK_TEXT}`, completed: true });
      actions.set(`${owner}-pick`, () => pickList(mogId, owner));
      actions.set(`${owner}-insert`, () => insert(mogId, owner, text));
      stack.offerChip({
        id: `${owner}-pick`,
        kind: 'choose_profile',
        label: 'Pick a profile',
        owner,
      });
      // [Insert as text] is the dictation action of the Appendix E `copy` row (ADR 0092). Both
      // chips expire after 30 s; the text stays on the card.
      stack.offerChip({ id: `${owner}-insert`, kind: 'copy', label: 'Insert as text', owner });
      stack.offerChip({ id: `${owner}-copy`, kind: 'copy', label: 'Copy', owner });
    },
    async chip(_mogId, action) {
      await actions.get(action.id)?.();
    },
  };
}
