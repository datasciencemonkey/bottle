/** V2 MOG view (thin): one mascot instance, badges and DOM overlays from `mog.view_state`. */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { MogId } from '../../shared/interfaces';
import { asMogViewState, emitView, useViewEvent, type MogViewState } from '../view-ipc';
import { PlaceholderMascot, type MascotRenderer } from './mascot';

export interface MogViewProps {
  mogId: MogId;
  /** The renderer seam (ADR 0002). TODO(#12): the Rive renderer. */
  createMascot?: () => MascotRenderer;
}

const placeholder = () => new PlaceholderMascot();

export function MogView({ mogId, createMascot = placeholder }: MogViewProps) {
  const ref = useRef<HTMLDivElement>(null);
  const mascot = useRef<MascotRenderer | undefined>(undefined);
  const [state, setState] = useState<MogViewState | undefined>(undefined);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const m = createMascot();
    m.mount(el);
    mascot.current = m;
    return () => {
      m.destroy();
      mascot.current = undefined;
    };
  }, [createMascot]);

  const onState = useCallback(
    (p: unknown) => {
      const s = asMogViewState(p);
      if (!s || s.mogId !== mogId) return;
      mascot.current?.setMood(s.mood);
      if (s.level !== undefined) mascot.current?.setLevel(s.level);
      setState(s);
    },
    [mogId],
  );
  useViewEvent('mog.view_state', onState);

  // TODO(#83): poke, click-to-listen and drag rules; the brain decides what input means.
  return (
    <div>
      <div
        ref={ref}
        aria-label="MOG"
        onClick={() => void emitView(mogId, 'click')}
        onMouseOver={() => void emitView(mogId, 'hover')}
      />
      {state?.overlays.attention && <span data-overlay="attention" aria-label="Needs attention" />}
      {state?.overlays.recording && <span data-overlay="recording">REC</span>}
      {state?.overlays.offline && <span data-overlay="offline">Offline</span>}
    </div>
  );
}
