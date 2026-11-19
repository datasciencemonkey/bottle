/**
 * shadcn/ui Input (review of #605), copied in as source and adapted to this repo's strict
 * TypeScript config. Source: ui.shadcn.com/docs/components/input (MIT). See
 * THIRD_PARTY_NOTICES.md. Every `type` gets the same box (hub.css's legacy field rule matches
 * only `text` and no type, so a `type="url"` field had no box).
 */
import type { ComponentProps } from 'react';
import { cn } from './utils';

function Input({ className, type, ...props }: ComponentProps<'input'>) {
  return (
    <input
      type={type}
      data-slot="input"
      className={cn(
        'border-input placeholder:text-muted-foreground h-9 w-full min-w-0 rounded-md border bg-transparent px-3 py-1 text-base shadow-xs transition-[color,box-shadow] motion-reduce:transition-none outline-none disabled:cursor-not-allowed disabled:opacity-50 md:text-sm',
        'focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]',
        className,
      )}
      {...props}
    />
  );
}

export { Input };
