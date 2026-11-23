/**
 * The workspace colour swatches (moved out of the old MOG list, #ui-4). `ColorDot` shows one
 * palette colour; `WorkspaceColor` is the picker: the fixed palette (`mog-palette.ts`) as a group
 * of toggle buttons. A pick calls `onPick` with the palette id, which the caller sends as
 * `workspace.edit` with `colorVariant`. Colours are the `--mog-palette-<id>` tokens, never a raw
 * hex (ADR 0106).
 */
import { MOG_PALETTE, paletteColorOf } from '../../shared/mog-palette';
import { cn } from '../ui/utils';

/** One round swatch. The id of the palette colour rides on `data-color`. */
export function ColorDot({
  colorVariant,
  profile,
  className,
}: {
  colorVariant: string;
  profile: string;
  className?: string;
}) {
  const color = paletteColorOf(colorVariant, profile);
  return (
    <span
      aria-hidden="true"
      data-color={color.id}
      className={cn(
        'inline-block size-3.5 shrink-0 rounded-full ring-1 ring-foreground/25',
        className,
      )}
      style={{ background: `var(--mog-palette-${color.id})` }}
    />
  );
}

export function WorkspaceColor({
  name,
  profile,
  colorVariant,
  onPick,
}: {
  name: string;
  profile: string;
  colorVariant: string;
  onPick: (colorVariant: string) => void;
}) {
  const current = paletteColorOf(colorVariant, profile);
  return (
    <div
      role="group"
      aria-label={`Colour for ${name}`}
      data-workspace-color
      className="flex flex-wrap gap-[var(--mog-space-2)]"
    >
      {MOG_PALETTE.map((c) => (
        <button
          key={c.id}
          type="button"
          data-slot="swatch"
          data-color={c.id}
          aria-label={c.label}
          aria-pressed={c.id === current.id}
          onClick={() => onPick(c.id)}
          className="size-6 rounded-full ring-1 ring-foreground/25 outline-none transition-[box-shadow] motion-reduce:transition-none focus-visible:ring-[3px] focus-visible:ring-ring aria-pressed:ring-2 aria-pressed:ring-offset-2 aria-pressed:ring-offset-card aria-pressed:ring-ring"
          style={{ background: `var(--mog-palette-${c.id})` }}
        />
      ))}
    </div>
  );
}
