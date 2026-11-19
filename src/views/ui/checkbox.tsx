/**
 * shadcn/ui Checkbox (ui-1, ADR 0104), copied in as source. Source:
 * ui.shadcn.com/docs/components/checkbox (MIT), wraps @radix-ui/react-checkbox. See
 * THIRD_PARTY_NOTICES.md.
 */
import * as CheckboxPrimitive from '@radix-ui/react-checkbox';
import { CheckIcon } from 'lucide-react';
import type { ComponentProps } from 'react';
import { cn } from './utils';

function Checkbox({ className, ...props }: ComponentProps<typeof CheckboxPrimitive.Root>) {
  return (
    <CheckboxPrimitive.Root
      data-slot="checkbox"
      className={cn(
        'peer border-input data-[state=checked]:bg-primary data-[state=checked]:text-primary-foreground data-[state=checked]:border-primary focus-visible:ring-[3px] focus-visible:ring-ring/50 size-4 shrink-0 rounded-[4px] border shadow-xs outline-none transition-shadow disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...props}
    >
      <CheckboxPrimitive.Indicator
        data-slot="checkbox-indicator"
        className="flex items-center justify-center text-current transition-none"
      >
        <CheckIcon className="size-3.5" />
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  );
}

export { Checkbox };
