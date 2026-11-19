/**
 * shadcn/ui Separator (ui-1, ADR 0104), copied in as source. Source:
 * ui.shadcn.com/docs/components/separator (MIT), wraps @radix-ui/react-separator. See
 * THIRD_PARTY_NOTICES.md.
 */
import * as SeparatorPrimitive from '@radix-ui/react-separator';
import type { ComponentProps } from 'react';
import { cn } from './utils';

function Separator({
  className,
  orientation = 'horizontal',
  decorative = true,
  ...props
}: ComponentProps<typeof SeparatorPrimitive.Root>) {
  return (
    <SeparatorPrimitive.Root
      data-slot="separator"
      decorative={decorative}
      orientation={orientation}
      className={cn(
        'bg-border shrink-0 data-[orientation=horizontal]:h-px data-[orientation=horizontal]:w-full data-[orientation=vertical]:h-full data-[orientation=vertical]:w-px',
        className,
      )}
      {...props}
    />
  );
}

export { Separator };
