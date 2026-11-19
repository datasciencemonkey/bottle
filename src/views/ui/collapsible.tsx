/**
 * shadcn/ui Collapsible (ui-1, ADR 0104), copied in as source. Source:
 * ui.shadcn.com/docs/components/collapsible (MIT), wraps @radix-ui/react-collapsible. See
 * THIRD_PARTY_NOTICES.md.
 */
import * as CollapsiblePrimitive from '@radix-ui/react-collapsible';
import type { ComponentProps } from 'react';

function Collapsible({ ...props }: ComponentProps<typeof CollapsiblePrimitive.Root>) {
  return <CollapsiblePrimitive.Root data-slot="collapsible" {...props} />;
}

function CollapsibleTrigger({
  ...props
}: ComponentProps<typeof CollapsiblePrimitive.CollapsibleTrigger>) {
  return <CollapsiblePrimitive.CollapsibleTrigger data-slot="collapsible-trigger" {...props} />;
}

function CollapsibleContent({
  ...props
}: ComponentProps<typeof CollapsiblePrimitive.CollapsibleContent>) {
  return <CollapsiblePrimitive.CollapsibleContent data-slot="collapsible-content" {...props} />;
}

export { Collapsible, CollapsibleTrigger, CollapsibleContent };
