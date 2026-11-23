/**
 * The one-time "Choose your home workspace" dialog (ADR 0117, FLAG 5: Bottle never picks a home
 * silently). It shows only while the brain sends `homePick`. `suggested` starts selected, and the
 * pick is sent as `workspace.pick_home`. Built on Radix's Dialog like `WipeDialog`: focus trap,
 * Escape to close, focus back to where it was. It renders inline (no portal) with the existing
 * `.hub-dialog` surface. Escape and Not now hide it; the Workspace section offers "Choose…" to reopen it.
 */
import * as DialogPrimitive from '@radix-ui/react-dialog';
import { useId, useState } from 'react';
import { HUB_SCOPE, type HubWorkspaceList } from '../../shared/hub-workspaces';
import { Button } from '../ui/button';
import { RadioGroup, RadioGroupItem } from '../ui/radio-group';
import { emitView } from '../view-ipc';

type HomePick = NonNullable<HubWorkspaceList['homePick']>;

/**
 * `open` and `onDismiss` are the caller's: the hub keeps the dismissed state, so the Workspace
 * section's "Choose…" button can open the dialog again. `names` maps a profile to its name.
 */
export function HomePickDialog({
  homePick,
  names,
  open,
  onDismiss,
}: {
  homePick: HomePick | undefined;
  names?: ReadonlyMap<string, string> | undefined;
  open: boolean;
  onDismiss: () => void;
}) {
  if (!homePick || !open) return null;
  const key = `${homePick.suggested}|${homePick.candidates.join(',')}`;
  return <PickBody key={key} homePick={homePick} names={names} onDismiss={onDismiss} />;
}

function PickBody({
  homePick,
  names,
  onDismiss,
}: {
  homePick: HomePick;
  names: ReadonlyMap<string, string> | undefined;
  onDismiss: () => void;
}) {
  const [choice, setChoice] = useState(homePick.suggested);
  const id = useId();
  // The suggestion always shows, even if the brain left it out of the candidates.
  const candidates = homePick.candidates.includes(homePick.suggested)
    ? homePick.candidates
    : [homePick.suggested, ...homePick.candidates];
  const use = () => {
    void emitView(HUB_SCOPE, 'click', { action: 'workspace.pick_home', profile: choice }).catch(
      () => undefined,
    );
  };
  return (
    <DialogPrimitive.Root open onOpenChange={(open) => !open && onDismiss()}>
      <DialogPrimitive.Content
        data-home-pick-dialog
        className="hub-dialog"
        onPointerDownOutside={(e) => e.preventDefault()}
        onInteractOutside={(e) => e.preventDefault()}
      >
        <DialogPrimitive.Title asChild>
          <h2 className="hub-dialog-title">Choose your home workspace</h2>
        </DialogPrimitive.Title>
        <DialogPrimitive.Description className="text-[var(--mog-text-md)] text-muted-foreground">
          Plain questions go to your home workspace first. You can change it later.
        </DialogPrimitive.Description>
        <RadioGroup
          aria-label="Home workspace"
          className="mt-[var(--mog-space-4)] gap-[var(--mog-space-1)]"
          value={choice}
          onValueChange={setChoice}
        >
          {candidates.map((p, i) => {
            const rid = `${id}-${i}`;
            const label = names?.get(p) ?? p;
            return (
              <div
                key={p}
                className="flex items-center gap-[var(--mog-space-3)] rounded-[var(--radius-md)] px-[var(--mog-space-3)] py-[var(--mog-space-2)] hover:bg-accent/40"
              >
                <RadioGroupItem id={rid} value={p} data-profile={p} />
                <label htmlFor={rid} className="flex min-w-0 flex-1 cursor-pointer flex-col">
                  <span className="truncate text-[var(--mog-text-md)]">{label}</span>
                  {label !== p && (
                    <span className="truncate text-[var(--mog-text-xs)] text-muted-foreground">
                      {p}
                    </span>
                  )}
                </label>
              </div>
            );
          })}
        </RadioGroup>
        <div className="hub-actions">
          <Button type="button" data-home-pick-use onClick={use}>
            Use as home
          </Button>
          <Button type="button" variant="outline" data-home-pick-later onClick={onDismiss}>
            Not now
          </Button>
        </div>
      </DialogPrimitive.Content>
    </DialogPrimitive.Root>
  );
}
