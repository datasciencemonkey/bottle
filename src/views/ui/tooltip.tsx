/**
 * shadcn/ui Tooltip (ui-1, ADR 0106), copied in as source. Source:
 * ui.shadcn.com/docs/components/tooltip (MIT), wraps @radix-ui/react-tooltip. See
 * THIRD_PARTY_NOTICES.md.
 */
import * as TooltipPrimitive from '@radix-ui/react-tooltip';
import type { ComponentProps } from 'react';
import { cn } from './utils';

function TooltipProvider({
  delayDuration = 0,
  ...props
}: ComponentProps<typeof TooltipPrimitive.Provider>) {
  return (
    <TooltipPrimitive.Provider
      data-slot="tooltip-provider"
      delayDuration={delayDuration}
      {...props}
    />
  );
}

function Tooltip({ ...props }: ComponentProps<typeof TooltipPrimitive.Root>) {
  return (
    <TooltipProvider>
      <TooltipPrimitive.Root data-slot="tooltip" {...props} />
    </TooltipProvider>
  );
}

function TooltipTrigger({ ...props }: ComponentProps<typeof TooltipPrimitive.Trigger>) {
  return <TooltipPrimitive.Trigger data-slot="tooltip-trigger" {...props} />;
}

function TooltipContent({
  className,
  sideOffset = 4,
  children,
  ...props
}: ComponentProps<typeof TooltipPrimitive.Content>) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        data-slot="tooltip-content"
        sideOffset={sideOffset}
        className={cn(
          // No `animate-in`/`fade-in-0`/`zoom-in-95` (ui-1, review of #602 round 3, LOW): those
          // are `tw-animate-css` classes, a dependency this repo never installed, so they were
          // dead weight (no enter animation at all). Removing them is the smaller fix than
          // adding the dependency (and a THIRD_PARTY_NOTICES entry) for one tooltip's fade-in.
          'bg-popover text-popover-foreground z-50 w-fit origin-(--radix-tooltip-content-transform-origin) rounded-md border border-border px-3 py-1.5 text-xs shadow-md',
          className,
        )}
        {...props}
      >
        {children}
      </TooltipPrimitive.Content>
    </TooltipPrimitive.Portal>
  );
}

export { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider };
