/** V2 MOG view (thin): one mascot instance, badges and DOM overlays from `mog.view_state`. */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { MogId } from '../../shared/interfaces';
import {
  asMogViewState,
  emitView,
  useMogTarget,
  useViewEvent,
  viewMicLevels,
  type MogViewState,
} from '../view-ipc';
import { LevelMeter, type MicLevelSource } from './level-meter';
import type { FrameProbe } from './frame-probe';
import type { MascotRenderer } from './mascot';
import { RiveMascot } from './rive-mascot';

export interface MogViewProps {
  mogId: MogId;
  /** The renderer seam (ADR 0002, ADR 0027). */
  createMascot?: () => MascotRenderer;
  /** Mic frames for the level meter (UX-06a). Default: the `mog.level` frames of the brain. */
  levels?: MicLevelSource;
  /** The frame probe for the latency run (#13, ADR 0031). Only with the trace flag. */
  probe?: FrameProbe | undefined;
}

/** One Rive instance per MOG view, with the PNG fallback (UX-09b). */
export const defaultMascot = (): MascotRenderer => new RiveMascot();

/**
 * CAP-16a, UX-06a: the `mog.level` frames the brain sends to this MOG window (`LevelRouter`).
 * One stable source, so a render does not subscribe again.
 */
const VIEW_MIC_LEVELS: MicLevelSource = viewMicLevels();

export function MogView({
  mogId,
  createMascot = defaultMascot,
  levels = VIEW_MIC_LEVELS,
  probe,
}: MogViewProps) {
  const ref = useRef<HTMLDivElement>(null);
  const mascot = useRef<MascotRenderer | undefined>(undefined);
  const last = useRef<MogViewState | undefined>(undefined);
  const [state, setState] = useState<MogViewState | undefined>(undefined);
  // The pointer is on the MOG: the view shows the hover badges (MOG-07, UX-38).
  // TODO(#10): hover delivery on the non-activating panel (SP-01, ADR 0009).
  const [hovered, setHovered] = useState(false);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const m = createMascot();
    m.mount(el);
    // A new renderer starts from the last state the brain sent.
    const s = last.current;
    if (s) {
      m.setMood(s.mood);
      m.setAttention(s.overlays.attention);
    }
    mascot.current = m;
    return () => {
      m.destroy();
      mascot.current = undefined;
    };
  }, [createMascot]);

  // UX-06a / UX-06b: the view computes the level, and only for the mic this MOG owns.
  // A new renderer gets a new meter, so the meter never skips its first write.
  useEffect(() => {
    const meter = new LevelMeter(mogId);
    return levels.subscribe((frame) => {
      const v = meter.push(frame);
      if (v !== undefined) mascot.current?.setLevel(v);
    });
  }, [levels, mogId, createMascot]);

  const onState = useCallback(
    (p: unknown) => {
      const s = asMogViewState(p);
      if (!s || s.mogId !== mogId) return;
      mascot.current?.setMood(s.mood);
      // UX-03: the "?" cue over any mood; the renderer skips a value that did not change.
      mascot.current?.setAttention(s.overlays.attention);
      // UX-36: the mood is on the mascot now, so the next frame draws it.
      probe?.onApplied(s.mood, s.trace);
      last.current = s;
      setState(s);
    },
    [mogId, probe],
  );
  useViewEvent('mog.view_state', onState);
  // MB-06b, UX-28 (#73): the brain says which MOG the talk key targets and when to show the toast.
  const target = useMogTarget(mogId);
  const name = target?.name ?? 'MOG';

  // TODO(#83): poke, click-to-listen and drag rules; the brain decides what input means.
  // The hover flag is on the whole view, so moving onto a badge keeps the hover badges open.
  return (
    <div onMouseOver={() => setHovered(true)} onMouseLeave={() => setHovered(false)}>
      <div
        ref={ref}
        className="mascot"
        aria-label={target?.target ? `${name}, target` : name}
        onClick={() => {
          mascot.current?.poke();
          void emitView(mogId, 'click');
        }}
        onMouseOver={() => {
          mascot.current?.setHover(true);
          void emitView(mogId, 'hover');
        }}
        onMouseLeave={() => {
          mascot.current?.setHover(false);
        }}
      />
      {/* UX-28: a triangle under the target mascot; the accessible name says "target". */}
      {target?.target && <span className="target-marker" data-marker="target" aria-hidden="true" />}
      {/* MB-06b: the brain shows the toast for 1.2 s after the talk key goes down. */}
      {target?.target && target.toast && (
        <span className="target-toast" data-toast="target" role="status">
          {`Talking to ${target.name}`}
        </span>
      )}
      {/* The visual "?" cue is the renderer's (UX-03); this empty span is the accessible badge. */}
      {state?.overlays.attention && <span data-overlay="attention" aria-label="Needs attention" />}
      {state?.overlays.recording && <span data-overlay="recording">REC</span>}
      {/* A payload with no badge list shows the 4.7 offline overlay; with one, the list wins. */}
      {state?.overlays.offline && state.badges === undefined && (
        <span data-overlay="offline">Offline</span>
      )}
      {/* The brain picks the badges and their text (ADR 0029); badges never set the mood. */}
      {state?.badges && (
        <div className="badges">
          {state.badges
            .filter((b) => hovered || !b.hoverOnly)
            .map((b, i) => {
              const body = (
                <>
                  <span data-badge-text>{b.text}</span>
                  {b.detail !== undefined && <span data-badge-detail>{b.detail}</span>}
                </>
              );
              const key = `${i}-${b.kind}`;
              return b.action !== undefined ? (
                <button
                  key={key}
                  type="button"
                  data-badge={b.kind}
                  aria-label={b.label}
                  onClick={() => void emitView(mogId, 'click', { target: 'badge', badge: b.kind })}
                >
                  {body}
                </button>
              ) : (
                <span key={key} role="note" data-badge={b.kind} aria-label={b.label}>
                  {body}
                </span>
              );
            })}
        </div>
      )}
    </div>
  );
}
