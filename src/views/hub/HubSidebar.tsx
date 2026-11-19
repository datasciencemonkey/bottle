/**
 * The hub's collapsible sidebar (ui-1, #97, #103; course-corrected to hidden-by-default, see
 * use-hub-sidebar.ts). An overlay over the content column, not a push layout, so the content
 * column's width never jumps when the sidebar opens.
 *
 * Motion only animates the width/slide; `useReducedMotion` turns the transition into an instant
 * show/hide (ui-1, ADR 0104). The active item's highlight uses a shared `layoutId`
 * ("hand-built on Motion; Animate UI/Magic UI copy-in deferred to ui-1b", per ui-lead).
 */
import { motion, useReducedMotion } from 'motion/react';
import { PanelLeftIcon, PinIcon, PinOffIcon } from 'lucide-react';
import { useEffect, useRef } from 'react';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '../ui/tooltip';
import { Button } from '../ui/button';
import { NAV_ITEMS, type HubView } from './nav';
import type { HubSidebarState } from './use-hub-sidebar';

const EXPANDED_WIDTH = 220;
const COLLAPSED_WIDTH = 56;

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
      if (e.key === 'Escape') requestClose();
    };
    const onPointerDown = (e: PointerEvent) => {
      if (panelRef.current && !panelRef.current.contains(e.target as Node)) requestClose();
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
            className="bg-popover text-popover-foreground absolute top-0 left-0 z-20 flex h-full flex-col gap-[var(--mog-space-1)] overflow-hidden border-r border-border/70 p-[var(--mog-space-2)] shadow-lg"
            initial={reduceMotion ? false : { width: 0, opacity: 0 }}
            animate={{ width, opacity: 1 }}
            transition={
              reduceMotion ? { duration: 0 } : { duration: 0.18, ease: [0.16, 1, 0.3, 1] }
            }
          >
            {/* No repeated "Bottle" label here: the hub header already shows it (AT-37). */}
            <div className="flex items-center justify-end px-[var(--mog-space-1)] py-[var(--mog-space-1)]">
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
                      'relative flex h-auto min-h-0 w-full items-center gap-[var(--mog-space-3)] rounded-[var(--radius-sm)] border-0 bg-transparent px-[var(--mog-space-2)] py-[var(--mog-space-2)] text-[var(--mog-text-sm)] transition-colors hover:text-foreground ' +
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
