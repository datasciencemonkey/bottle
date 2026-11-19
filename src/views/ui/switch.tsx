/**
 * shadcn/ui Switch (ui-1, ADR 0106), copied in as source. Source:
 * ui.shadcn.com/docs/components/switch (MIT), wraps @radix-ui/react-switch. See
 * THIRD_PARTY_NOTICES.md.
 */
import * as SwitchPrimitive from '@radix-ui/react-switch';
import type { ComponentProps } from 'react';
import { cn } from './utils';

function Switch({ className, ...props }: ComponentProps<typeof SwitchPrimitive.Root>) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      className={cn(
        'peer data-[state=checked]:bg-primary data-[state=unchecked]:bg-input focus-visible:ring-[3px] focus-visible:ring-ring/50 inline-flex h-[1.15rem] w-8 shrink-0 items-center rounded-full border border-transparent shadow-xs transition-all motion-reduce:transition-none outline-none disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...props}
    >
      {/* bg-foreground, not bg-background (ui-1, review of #602 finding 5): a thumb in the
          page's own colour was invisible against an unchecked track of the same surface. */}
      <SwitchPrimitive.Thumb
        data-slot="switch-thumb"
        className={cn(
          'bg-foreground pointer-events-none block size-4 rounded-full ring-0 transition-transform motion-reduce:transition-none data-[state=checked]:translate-x-[calc(100%-2px)] data-[state=unchecked]:translate-x-0',
        )}
      />
    </SwitchPrimitive.Root>
  );
}

export { Switch };
