/**
 * Hop proposals wait for the user (#141, ADR 0085; RTR-29d, F5 step 4, UX-20c).
 *
 * The router step is the main agent's first step (B4). When it proposes a hop (#144,
 * `DispatchOutcome` `hop`), the MOG shows the "Switch to <profile>?" chip (Appendix E
 * `switch_profile`, 30 s) and waits. Only a click on that chip calls `profile_select` with the
 * proposed profile and scope. An offer, an expiry, a dismiss or any agent call switches nothing.
 * The agent has no handle to this class's `click`: the card's click handler calls it.
 *
 * TODO(#165): the hop execution after `profile_select` (badge, banner, new conversations).
 */
import type { MogId } from '../../shared/interfaces';
import { call, type CommandArgs } from '../../shared/ipc';
import type { ChipRow } from '../card-chips';
import type { DispatchOutcome, HopProposal } from './router-dispatch';

export interface HopChipsDeps {
  /** The chip row of the MOG card. */
  chips: ChipRow;
  /** The monotonic time in ms. */
  now: () => number;
  /** The `profile_select` call. Default: the IPC `call`. */
  select?: (args: CommandArgs['profile_select']) => Promise<unknown>;
}

interface Pending {
  mogId: MogId;
  proposal: HopProposal;
}

let seq = 0;

export class HopChips {
  private readonly pending = new Map<string, Pending>();
  private readonly select: (args: CommandArgs['profile_select']) => Promise<unknown>;

  constructor(private readonly deps: HopChipsDeps) {
    this.select = deps.select ?? ((a) => call('profile_select', a));
  }

  /** Show the chip for `proposal`. Returns the chip ID, or undefined if the row refused it. */
  offer(mogId: MogId, proposal: HopProposal): string | undefined {
    if (proposal.requiresUserAction !== true || proposal.profile.trim() === '') return undefined;
    const id = `hop-${++seq}`;
    const ok = this.deps.chips.offer(
      { id, kind: 'switch_profile', label: `Switch to ${proposal.profile}?` },
      this.deps.now(),
    );
    if (!ok) return undefined;
    this.pending.set(id, { mogId, proposal: { ...proposal } });
    return id;
  }

  /** Offer the chip for a `hop` outcome. Every other outcome shows nothing here. */
  offerOutcome(mogId: MogId, o: DispatchOutcome): string | undefined {
    return o.kind === 'hop' ? this.offer(mogId, o.proposal) : undefined;
  }

  /** The user closed the chip: no switch. */
  dismiss(id: string): void {
    this.pending.delete(id);
    this.deps.chips.remove(id);
  }

  /**
   * A click on the chip (UX-20c): the user's approval. True if it called `profile_select`.
   * A chip that expired, was dismissed or is unknown gives false and calls nothing.
   */
  async click(id: string): Promise<boolean> {
    const p = this.pending.get(id);
    this.pending.delete(id);
    const action = this.deps.chips.click(id);
    if (p === undefined || action?.kind !== 'switch_profile') return false;
    this.deps.chips.remove(id);
    await this.select({ mogId: p.mogId, profile: p.proposal.profile, scope: p.proposal.scope });
    return true;
  }
}
