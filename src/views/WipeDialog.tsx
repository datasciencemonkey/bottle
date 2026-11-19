/**
 * The Wipe dialog (thin; MEM-33a, G-45b, AT-16; ADR 0087). The brain builds the model
 * (`src/brain/components/wipe-flow.ts`). The confirm button sends `view_emit` with the option
 * only; the brain runs the Wipe. After the delete, the dialog mounts one "Delete in Genie One"
 * notice for each MOG (G-45b). Every click goes to the brain (`src/brain/wipe-wiring.ts`), which
 * owns the option and the links. TODO(#87): the brain opens a link with the G-19a opener.
 *
 * ui-1 (ADR 0106): rebuilt on Radix's Dialog primitive (focus trap, Escape-to-close) instead of a
 * plain positioned `<div>`. It renders inline (no `DialogPrimitive.Portal`): the hub's dialogs
 * (ForgetDialog, this one) have always lived in the same document as the rest of the hub, and a
 * portal would move the markup outside the element every test scopes its queries to. The scrim is
 * the existing `.hub-dialog` CSS (a giant `box-shadow`, #561), not a separate overlay element.
 */
import * as DialogPrimitive from '@radix-ui/react-dialog';
import {
  WIPE_CANCEL_TARGET,
  WIPE_CONFIRM_TARGET,
  WIPE_VIEW_ID,
  type WipeDialogModel,
} from '../shared/wipe-view';
import { Button } from './ui/button';
import { GenieOneDeleteNotice } from './GenieOneDeleteNotice';
import { emitView } from './view-ipc';

export function WipeDialog({ model }: { model: WipeDialogModel }) {
  const { result } = model;
  const cancel = () => void emitView(WIPE_VIEW_ID, 'click', { target: WIPE_CANCEL_TARGET });

  return (
    <DialogPrimitive.Root
      open
      onOpenChange={(open) => {
        if (!open) cancel();
      }}
    >
      <DialogPrimitive.Content
        data-wipe-dialog
        className="hub-dialog"
        onEscapeKeyDown={cancel}
        onPointerDownOutside={(e) => e.preventDefault()}
      >
        <DialogPrimitive.Title asChild>
          <h2 className="hub-dialog-title">{model.title}</h2>
        </DialogPrimitive.Title>
        {model.phase === 'confirm' && (
          <>
            <p>{model.text}</p>
            <div className="hub-actions">
              <Button
                type="button"
                variant="destructive"
                data-wipe-confirm
                onClick={() =>
                  void emitView(WIPE_VIEW_ID, 'click', {
                    target: WIPE_CONFIRM_TARGET,
                    scope: model.scope,
                  })
                }
              >
                Wipe
              </Button>
              <Button type="button" variant="outline" data-wipe-cancel onClick={cancel}>
                Cancel
              </Button>
            </div>
          </>
        )}
        {model.phase === 'done' && result !== undefined && (
          <>
            <p data-wipe-result>
              Bottle deleted {result.local} {result.local === 1 ? 'row' : 'rows'} on this computer
              and {result.deletedConversations} Genie{' '}
              {result.deletedConversations === 1 ? 'conversation' : 'conversations'}.
            </p>
            {result.notices.map((n) => (
              <GenieOneDeleteNotice key={n.mogId} notice={n} />
            ))}
            {result.otherText !== undefined && <p data-wipe-other>{result.otherText}</p>}
            <div className="hub-actions">
              <Button type="button" variant="outline" data-wipe-cancel onClick={cancel}>
                Close
              </Button>
            </div>
          </>
        )}
      </DialogPrimitive.Content>
    </DialogPrimitive.Root>
  );
}
