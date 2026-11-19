/**
 * A native macOS settings pane's grouped list (ui-1, review of #602 "also fix": "Settings must
 * be grouped rows... labels left, controls right, rounded group containers"). `SettingsGroup` is
 * the rounded box; `SettingsRow` is one row inside it, a label (and optional helper text) on the
 * left and a control on the right. Hand-written, hub-specific layout, not a shadcn/ui copy-in
 * (like `HubSidebar.tsx`).
 *
 * `bg-card` (design-grammar pass, stageshow skill, "Color and looks"): on a dark theme, depth
 * comes from a lighter surface, not a shadow or an outline border alone — a border with no fill
 * reads as a thin outline floating on the same surface as the page, not a grouped, raised
 * native list. `bg-card` is `--mog-surface-raised`, the same raised tone the sidebar panel
 * (`bg-popover`) already uses, so the group reads as one step up from the page without
 * introducing a second surface colour.
 */
import type { ReactNode } from 'react';
import { cn } from '../ui/utils';

export function SettingsGroup({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        'bg-card divide-y divide-border/60 overflow-hidden rounded-[var(--radius-md)] border border-border/60',
        className,
      )}
    >
      {children}
    </div>
  );
}

export function SettingsRow({
  label,
  description,
  control,
  htmlFor,
  className,
}: {
  label: ReactNode;
  description?: ReactNode;
  control: ReactNode;
  /** Associates the row's label with `control`'s id, so clicking the label toggles it too. */
  htmlFor?: string;
  className?: string;
}) {
  return (
    <div
      className={cn(
        'flex items-center justify-between gap-[var(--mog-space-3)] px-[var(--mog-space-4)] py-[var(--mog-space-3)]',
        className,
      )}
    >
      <label
        htmlFor={htmlFor}
        className={cn(
          'flex min-w-0 flex-col gap-[var(--mog-space-1)]',
          htmlFor ? 'cursor-pointer' : undefined,
        )}
      >
        <span className="text-[var(--mog-text-md)]">{label}</span>
        {description !== undefined && (
          <span className="text-[var(--mog-text-xs)] text-muted-foreground">{description}</span>
        )}
      </label>
      <div className="shrink-0">{control}</div>
    </div>
  );
}
