/**
 * shadcn/ui Button (ui-1, ADR 0104), copied in as source and adapted to this repo's strict
 * TypeScript config (verbatimModuleSyntax, exactOptionalPropertyTypes). Source:
 * ui.shadcn.com/docs/components/button (MIT). See THIRD_PARTY_NOTICES.md.
 */
import { Slot } from '@radix-ui/react-slot';
import { cva, type VariantProps } from 'class-variance-authority';
import type { ComponentProps } from 'react';
import { cn } from './utils';

const buttonVariants = cva(
  // border-0 (ui-1, ADR 0104): hub.css's legacy `.hub button { border: 1px solid ... }` (#561)
  // sits in a lower-priority cascade layer than this component's Tailwind utilities, but a
  // property only loses a layer fight when something in a higher layer actually sets it; with
  // no border utility of its own, a shadcn button still picked up the legacy hairline. The
  // `outline` variant below re-adds `border` deliberately.
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md border-0 text-sm font-medium outline-none transition-[color,background-color,border-color,box-shadow] disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4 focus-visible:ring-[3px] focus-visible:ring-ring/50",
  {
    variants: {
      variant: {
        default: 'bg-primary text-primary-foreground shadow-xs hover:bg-primary/90',
        destructive:
          'bg-destructive text-white shadow-xs hover:bg-destructive/90 focus-visible:ring-destructive/40',
        outline: 'border bg-transparent shadow-xs hover:bg-accent hover:text-accent-foreground',
        secondary: 'bg-secondary text-secondary-foreground shadow-xs hover:bg-secondary/80',
        ghost: 'hover:bg-accent hover:text-accent-foreground',
        link: 'text-primary underline-offset-4 hover:underline',
      },
      size: {
        default: 'h-9 px-4 py-2 has-[>svg]:px-3',
        sm: 'h-8 gap-1.5 px-3 has-[>svg]:px-2.5',
        lg: 'h-10 px-6 has-[>svg]:px-4',
        icon: 'size-9',
      },
    },
    defaultVariants: {
      variant: 'default',
      size: 'default',
    },
  },
);

function Button({
  className,
  variant,
  size,
  asChild = false,
  ...props
}: ComponentProps<'button'> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean;
  }) {
  const Comp = asChild ? Slot : 'button';
  return (
    <Comp
      data-slot="button"
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  );
}

export { Button, buttonVariants };
