/**
 * shadcn/ui Sonner toaster (ui-1, ADR 0104), copied in as source. Source:
 * ui.shadcn.com/docs/components/sonner (MIT), wraps the `sonner` package. See
 * THIRD_PARTY_NOTICES.md. Themed dark-only (the hub stays dark-first, #561).
 */
import { Toaster as Sonner, type ToasterProps } from 'sonner';

function Toaster({ ...props }: ToasterProps) {
  return (
    <Sonner
      theme="dark"
      className="toaster group"
      style={
        {
          '--normal-bg': 'var(--popover)',
          '--normal-text': 'var(--popover-foreground)',
          '--normal-border': 'var(--border)',
        } as React.CSSProperties
      }
      {...props}
    />
  );
}

export { Toaster };
