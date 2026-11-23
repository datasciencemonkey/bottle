/**
 * The hub's collapsible sidebar (ui-1, #97, #103; course-corrected to hidden-by-default, see
 * use-hub-sidebar.ts). The panel is `position: fixed; inset-y-0; left-0` (ui-1, review of #602
 * round 3, blocker 2): a full-window-height rail pinned to the window itself, not to any
 * ancestor box, so it never scrolls away with a tall Settings area and its own top row (the
 * pin button) always starts at the window's true top edge, not below the "Bottle" header. Before
 * this it was `position: absolute; top: 0; h-full` inside `.hub-body` (hub.css): that box sits
 * below the header in the document, and grows to the page's own (unbounded) height, so the panel
 * both started too low and scrolled off-screen with the page.
 *
 * Unpinned, it still overlays the content column with no reflow, and keeps its soft `shadow-lg`
 * (an overlay floats above the page). Pinned, `Hub.tsx` reads `SIDEBAR_WIDTH` below to push
 * `.hub-content` over by the same amount (`--hub-sidebar-width` in hub.css), so a pinned panel
 * never covers the content for good (review of #602, finding 3) — and drops the shadow (review
 * of #602 round 3, polish): a pinned rail is flat with a border, not a popover; only an
 * unpinned overlay floats.
 *
 * Motion only animates the width/slide; `useReducedMotion` turns the transition into an instant
 * show/hide (ui-1, ADR 0106). The active item's highlight uses a shared `layoutId`
 * ("hand-built on Motion; Animate UI/Magic UI copy-in deferred to ui-1b", per ui-lead).
 */
import { motion, useReducedMotion } from 'motion/react';
import { PanelLeftIcon, PinIcon, PinOffIcon } from 'lucide-react';
import { useEffect, useRef } from 'react';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '../ui/tooltip';
import { Button } from '../ui/button';
import { NAV_ITEMS, type HubView } from './nav';
import type { HubSidebarState } from './use-hub-sidebar';

export const EXPANDED_WIDTH = 220;
export const COLLAPSED_WIDTH = 56;

/** The current sidebar width in px, or 0 while it is not pinned open (Hub.tsx, hub.css). */
export function sidebarWidth(state: Pick<HubSidebarState, 'pinned' | 'visible' | 'collapsed'>) {
  if (!state.pinned || !state.visible) return 0;
  return state.collapsed ? COLLAPSED_WIDTH : EXPANDED_WIDTH;
}

export function HubSidebar({
  state,
  view,
  onView,
}: {
  state: HubSidebarState;
  view: HubView;
  onView: (v: HubView) => void;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  const reduceMotion = useReducedMotion();
  const { visible, collapsed, pinned, togglePinned, toggleCollapsed, requestClose } = state;

  // Esc and a click outside the panel hide it, unless pinned open (use-hub-sidebar.ts).
  useEffect(() => {
    if (!visible) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      // LOW (review of #602 round 3): the Wipe dialog (Radix Dialog, `role="dialog"`) owns Esc
      // while it is open (`WipeDialog.tsx`'s own `onEscapeKeyDown` cancels it) — without this
      // check, this document-level listener raced it and closed the sidebar underneath the
      // still-open dialog.
      if (document.querySelector('[role="dialog"]')) return;
      requestClose();
    };
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Element | null;
      if (panelRef.current?.contains(target)) return;
      // review of #602, finding 2: the header toggle sits outside the panel, so a pointerdown
      // on it used to count as an outside click and close the sidebar; the `click` that follows
      // then reopened it (toggleVisible flips the now-closed state back open), so the toggle
      // looked like it did nothing. Skip it here and let its own onClick (toggleVisible) decide.
      if (target?.closest('[data-sidebar-toggle]')) return;
      requestClose();
    };
    document.addEventListener('keydown', onKeyDown);
    document.addEventListener('pointerdown', onPointerDown);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
      document.removeEventListener('pointerdown', onPointerDown);
    };
  }, [visible, requestClose]);

  const width = collapsed ? COLLAPSED_WIDTH : EXPANDED_WIDTH;

  return (
    <TooltipProvider>
      {
        // Plain conditional rendering, not AnimatePresence: closing (Esc, outside click, the
        // toggle) removes the panel immediately, so the content column's keyboard focus and a
        // test's next assertion never race a close animation. Opening still slides/fades in via
        // `initial`/`animate` (reduced motion: instant, `initial={false}`).
        visible && (
          <motion.nav
            ref={panelRef}
            aria-label="Hub sidebar"
            data-sidebar
            data-collapsed={collapsed ? '' : undefined}
            className={
              'bg-popover text-popover-foreground fixed inset-y-0 left-0 z-20 flex flex-col gap-[var(--mog-space-1)] overflow-hidden border-r border-border/70 p-[var(--mog-space-2)] ' +
              (pinned ? '' : 'shadow-lg')
            }
            initial={reduceMotion ? false : { width: 0, opacity: 0 }}
            animate={{ width, opacity: 1 }}
            transition={
              reduceMotion ? { duration: 0 } : { duration: 0.18, ease: [0.16, 1, 0.3, 1] }
            }
          >
            {/* No repeated "Bottle" label here: the hub header already shows it (AT-37). h-10,
                not an extra py on top of the panel's own padding (review polish note): lines
                the pin button's vertical centre up with the content column's area-header row
                (hub-area-header in Hub.tsx), which has no top padding of its own either. */}
            <div className="flex h-10 items-center justify-end px-[var(--mog-space-1)]">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="size-6"
                    data-sidebar-pin
                    aria-pressed={pinned}
                    onClick={togglePinned}
                  >
                    {pinned ? (
                      <PinOffIcon className="size-3.5" />
                    ) : (
                      <PinIcon className="size-3.5" />
                    )}
                    <span className="sr-only">{pinned ? 'Unpin sidebar' : 'Pin sidebar open'}</span>
                  </Button>
                </TooltipTrigger>
                <TooltipContent side="right">
                  {pinned ? 'Unpin: Esc or outside click will hide it' : 'Pin open'}
                </TooltipContent>
              </Tooltip>
            </div>
            <ul className="flex flex-col gap-[var(--mog-space-1)]" role="list">
              {NAV_ITEMS.map((item) => {
                const Icon = item.icon;
                const active = item.id === view;
                const button = (
                  <button
                    type="button"
                    aria-current={active ? 'page' : undefined}
                    data-sidebar-item={item.id}
                    onClick={() => onView(item.id)}
                    className={
                      'relative flex h-auto min-h-0 w-full items-center gap-[var(--mog-space-3)] rounded-[var(--radius-sm)] border-0 bg-transparent px-[var(--mog-space-2)] py-[var(--mog-space-2)] text-[var(--mog-text-sm)] transition-colors motion-reduce:transition-none hover:text-foreground ' +
                      (active ? 'text-foreground' : 'text-muted-foreground')
                    }
                  >
                    {active && (
                      <motion.span
                        layoutId="hub-sidebar-active"
                        className="absolute inset-0 rounded-[var(--radius-sm)] bg-accent"
                        {...(reduceMotion ? { transition: { duration: 0 } } : {})}
                      />
                    )}
                    <Icon className="relative z-10 size-4 shrink-0" />
                    {!collapsed && <span className="relative z-10">{item.label}</span>}
                  </button>
                );
                return (
                  <li key={item.id}>
                    {collapsed ? (
                      <Tooltip>
                        <TooltipTrigger asChild>{button}</TooltipTrigger>
                        <TooltipContent side="right">{item.label}</TooltipContent>
                      </Tooltip>
                    ) : (
                      button
                    )}
                  </li>
                );
              })}
            </ul>
            <div className="mt-auto flex justify-center">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="size-6"
                    data-sidebar-collapse
                    onClick={toggleCollapsed}
                  >
                    <PanelLeftIcon className="size-3.5" />
                    <span className="sr-only">
                      {collapsed ? 'Expand sidebar' : 'Collapse to icons'}
                    </span>
                  </Button>
                </TooltipTrigger>
                <TooltipContent side="right">
                  {collapsed ? 'Expand' : 'Collapse to icons'}
                </TooltipContent>
              </Tooltip>
            </div>
          </motion.nav>
        )
      }
    </TooltipProvider>
  );
}
