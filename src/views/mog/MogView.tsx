/** V2 MOG view (thin): one mascot instance, badges and DOM overlays from `mog.view_state`. */
import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { MOG_MENU_HIDE_EVENT, MOG_MENU_OPEN, OPEN_CONVERSATIONS } from '../../shared/genie-status';
import type { MogId } from '../../shared/interfaces';
import { paletteColorOf } from '../../shared/mog-palette';
import { MOG_LOOK_EVENT, MOG_VIEW_HIDDEN, asMogLook } from '../../shared/mog-look';
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

/**
 * #573: a single click waits this long, so a double-click (open hub Conversations) never also
 * runs the single-click action (start or stop listening).
 */
export const DOUBLE_CLICK_MS = 250;

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
  // #573: the brain holds the Genie card while the pointer is on the MOG (`hover { on }`).
  const pointerOn = useRef(false);
  const clickTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(clickTimer.current), []);
  const pointer = (on: boolean) => {
    setHovered(on);
    if (pointerOn.current === on) return;
    pointerOn.current = on;
    void emitView(mogId, 'hover', { on }).catch(() => undefined);
  };

  // #ui-4: Hide sinks the Genie into the lamp and shrinks the lamp art to lamp-only inside the
  // same 160x168 window (plain CSS, no brain round trip for showing or hiding the art: this
  // view holds the flag, so a click brings it back at once). It keeps running and stays a talk
  // target either way (review #603, LOW finding: the window itself never resizes, only the art).
  const [hiddenByUser, setHiddenByUser] = useState(false);
  // #ui-4 (review #603, findings 1 and 4): the right-click menu is a native Tauri menu, but a
  // MOG view may only listen to events and call `view_emit` (ARC-4) — only the brain holds
  // `core:menu:*`. This view only asks the brain to show it (`mog-context-menu.ts` builds and
  // pops it up on the real OS, which always fits the screen and dismisses on an outside click or
  // Esc with no listener needed here) and, for Hide only, is told the result back.
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void listen<{ hidden?: unknown }>(
      MOG_MENU_HIDE_EVENT,
      (e) => {
        const hidden = (e.payload as { hidden?: unknown } | null)?.hidden;
        if (typeof hidden === 'boolean') setHiddenByUser(hidden);
      },
      { target: getCurrentWindow().label },
    ).then((u) => {
      if (gone) u();
      else off = u;
    });
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  // lamp-alive: the host reads the global pointer and the brain sends this MOG where to look
  // (`mog:look`, a vector, never a position). The view only listens (ARC-4).
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void listen<unknown>(
      MOG_LOOK_EVENT,
      (e) => {
        const look = asMogLook(e.payload);
        if (look) mascot.current?.setLook?.(look.dx, look.dy);
      },
      { target: getCurrentWindow().label },
    )
      .then((u) => {
        if (gone) u();
        else off = u;
      })
      .catch(() => undefined);
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  // The host polls the pointer only for a MOG the user did not hide: tell it (it does not relay
  // this to the brain). Also once on load, so a reopened MOG is not left marked hidden.
  useEffect(() => {
    void emitView(mogId, 'click', { action: MOG_VIEW_HIDDEN, hidden: hiddenByUser }).catch(
      () => undefined,
    );
  }, [mogId, hiddenByUser]);

  // The idle motion (the spark orbit) and the gaze stop while the MOG is hidden: by the Hide menu
  // item, or because the page is not visible (a hidden window, a locked screen).
  const [pageHidden, setPageHidden] = useState(
    typeof document !== 'undefined' && document.visibilityState === 'hidden',
  );
  useEffect(() => {
    const on = () => setPageHidden(document.visibilityState === 'hidden');
    document.addEventListener('visibilitychange', on);
    return () => document.removeEventListener('visibilitychange', on);
  }, []);
  const idleStopped = hiddenByUser || pageHidden;
  const stopped = useRef(idleStopped);
  useEffect(() => {
    stopped.current = idleStopped;
    mascot.current?.setHidden?.(idleStopped);
  }, [idleStopped, createMascot]);

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
    m.setHidden?.(stopped.current);
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
  // #ui-4: the fixed, accessible palette (`mog-palette.ts`); an unknown or missing variant falls
  // back to a colour hashed from the MOG ID, so even a pre-identity render stays stable.
  const accent = paletteColorOf(target?.colorVariant, mogId);
  // #ui-4: a hover tooltip with the name and home workspace, if known (cheap: it rides the same
  // `mog:target` push, no extra round trip).
  const tooltip = target?.home ? `${name} · ${target.home}` : name;

  // TODO(#83): poke, click-to-listen and drag rules; the brain decides what input means.
  // The hover flag is on the whole view, so moving onto a badge keeps the hover badges open.
  return (
    <div
      style={{ '--mog-id-color': accent.hex } as CSSProperties}
      onMouseOver={() => pointer(true)}
      onMouseLeave={() => pointer(false)}
    >
      <div
        ref={ref}
        className="mascot"
        data-hidden={hiddenByUser || undefined}
        title={tooltip}
        aria-label={target?.target ? `${name}, target` : name}
        onContextMenu={(e) => {
          e.preventDefault();
          // `.mascot` fills the window (mog.css), so the click's viewport position is already
          // the window-relative position the brain's native popup wants (review #603, finding
          // 1: a native menu, unlike the old HTML one, always fits on screen regardless of
          // where this lands).
          void emitView(mogId, 'click', {
            target: MOG_MENU_OPEN,
            x: e.clientX,
            y: e.clientY,
            hidden: hiddenByUser,
          }).catch(() => undefined);
        }}
        onClick={(e) => {
          // The second click of a double-click is part of the double-click.
          if (e.detail > 1) return;
          // #ui-4: Hide is local-only; a click anywhere on the MOG brings it back.
          if (hiddenByUser) {
            setHiddenByUser(false);
            return;
          }
          mascot.current?.poke();
          clearTimeout(clickTimer.current);
          clickTimer.current = setTimeout(() => void emitView(mogId, 'click'), DOUBLE_CLICK_MS);
        }}
        onDoubleClick={() => {
          // #573: a user action: the hub opens on Conversations, and the card fades.
          clearTimeout(clickTimer.current);
          void emitView(mogId, 'click', { target: OPEN_CONVERSATIONS }).catch(() => undefined);
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
