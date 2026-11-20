/**
 * shadcn/ui Textarea (genie-followup, ADR 0106), copied in as source. Source:
 * ui.shadcn.com/docs/components/textarea (MIT). See THIRD_PARTY_NOTICES.md.
 * `min-h-0`, `border-input` and the explicit background override the legacy `.hub textarea` rule.
 */
import type { ComponentProps } from 'react';
import { cn } from './utils';

function Textarea({ className, ...props }: ComponentProps<'textarea'>) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        'border-input placeholder:text-muted-foreground min-h-9 w-full resize-none rounded-md border bg-transparent px-3 py-2 text-sm shadow-xs outline-none transition-[color,box-shadow] focus-visible:ring-[3px] focus-visible:ring-ring/50 motion-reduce:transition-none disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...props}
    />
  );
}

export { Textarea };
