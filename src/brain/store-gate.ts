/**
 * One local store open per launch, after the hub explains it (MEM-08b, ADR 0105).
 *
 * The local store key is in the OS key store. On macOS a build that the keychain item does not
 * trust yet (each new unsigned build) makes macOS ask for the login password on the first read.
 * The gate makes that one clear prompt:
 * - On macOS no store call runs until the hub shows the reason (`allow`), or the user presses the
 *   talk key.
 * - The first store call runs alone. The others wait for it, so a refusal never shows a second
 *   prompt.
 * - A refused key (`consent_storage_failed`, `host_failed`) turns history off. Each later store
 *   call fails at once, with no prompt, until the user selects Try again (`retry`).
 * Unarmed (tests, before `startBrain`), the gate passes every call through.
 */
import type { StoreModel } from '../shared/store-view';

export type StoreView = StoreModel;

/** The reply codes of a store that could not open (the key store refused or failed). */
const STORE_FAILED: ReadonlySet<string> = new Set(['consent_storage_failed', 'host_failed']);

const isStoreFailure = (e: unknown): boolean =>
  typeof e === 'object' && e !== null && STORE_FAILED.has(String((e as { error?: unknown }).error));

type State = 'waiting' | 'ready' | 'opening' | 'open' | 'off';

export interface StoreGate {
  /** Run one store call through the gate. */
  run<T>(fn: () => Promise<T>): Promise<T>;
  /** Start the gate. `explain`: wait for the hub (macOS). */
  arm(o: { explain: boolean; onChange: (v: StoreView) => void }): void;
  /** The hub shows the reason, or the user pressed the talk key: the store may open now. */
  allow(): void;
  /** Try again after a refusal. */
  retry(): void;
  /** The hub model now. */
  view(): StoreView;
}

export function createStoreGate(): StoreGate {
  let armed = false;
  let explain = false;
  let state: State = 'open';
  let failure: unknown;
  let onChange: (v: StoreView) => void = () => undefined;
  let release: () => void = () => undefined;
  let allowed: Promise<void> = Promise.resolve();
  /** The first call, while it opens the store. */
  let probe: Promise<unknown> | undefined;

  const view = (): StoreView => {
    if (state === 'off') return { state: 'off' };
    if (explain && (state === 'waiting' || state === 'ready' || state === 'opening'))
      return { state: 'explain' };
    return { state: 'hidden' };
  };
  const set = (s: State) => {
    state = s;
    onChange(view());
  };

  const run = async <T>(fn: () => Promise<T>): Promise<T> => {
    if (!armed) return fn();
    await allowed;
    for (;;) {
      if (state === 'open') return fn();
      if (state === 'off') throw failure;
      if (state === 'opening' && probe) {
        await probe.then(
          () => undefined,
          () => undefined,
        );
        continue;
      }
      // `ready`: this call opens the store.
      state = 'opening';
      const p = fn();
      probe = p;
      try {
        const out = await p;
        set('open');
        return out;
      } catch (e) {
        if (isStoreFailure(e)) {
          failure = e;
          set('off');
        } else {
          state = 'ready';
        }
        throw e;
      } finally {
        probe = undefined;
      }
    }
  };

  return {
    run,
    arm(o) {
      armed = true;
      explain = o.explain;
      onChange = o.onChange;
      if (explain) {
        allowed = new Promise<void>((r) => (release = r));
        set('waiting');
      } else {
        set('ready');
      }
    },
    allow() {
      if (state === 'waiting') state = 'ready';
      release();
    },
    retry() {
      if (state !== 'off') return;
      failure = undefined;
      // The hub showed the reason already: no new wait.
      explain = false;
      set('ready');
    },
    view,
  };
}

/** The brain's one gate. `startBrain` arms it. */
export const storeGate: StoreGate = createStoreGate();
