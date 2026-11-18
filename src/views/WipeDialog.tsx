/**
 * The Wipe dialog (thin; MEM-33a, G-45b, AT-16; ADR 0087). The brain builds the model
 * (`src/brain/components/wipe-flow.ts`). The confirm button sends `view_emit` with the option
 * only; the brain runs the Wipe. After the delete, the dialog mounts one "Delete in Genie One"
 * notice for each MOG (G-45b). Every click goes to the brain (`src/brain/wipe-wiring.ts`), which
 * owns the option and the links. TODO(#87): the brain opens a link with the G-19a opener.
 */
import type { WipeDialogModel } from '../shared/wipe-view';
import { WIPE_CANCEL_TARGET, WIPE_CONFIRM_TARGET, WIPE_VIEW_ID } from '../shared/wipe-view';
import { GenieOneDeleteNotice } from './GenieOneDeleteNotice';
import { emitView } from './view-ipc';

export function WipeDialog({ model }: { model: WipeDialogModel }) {
  const { result } = model;
  return (
    <div data-wipe-dialog role="dialog" aria-label={model.title}>
      <h2>{model.title}</h2>
      {model.phase === 'confirm' && (
        <>
          <p>{model.text}</p>
          <button
            type="button"
            data-wipe-confirm
            onClick={() =>
              void emitView(WIPE_VIEW_ID, 'click', {
                target: WIPE_CONFIRM_TARGET,
                scope: model.scope,
              })
            }
          >
            Wipe
          </button>
        </>
      )}
      {model.phase === 'done' && result !== undefined && (
        <>
          <p data-wipe-result>
            Bottle deleted {result.local} {result.local === 1 ? 'row' : 'rows'} on this computer and{' '}
            {result.deletedConversations} Genie{' '}
            {result.deletedConversations === 1 ? 'conversation' : 'conversations'}.
          </p>
          {result.notices.map((n) => (
            <GenieOneDeleteNotice key={n.mogId} notice={n} />
          ))}
          {result.otherText !== undefined && <p data-wipe-other>{result.otherText}</p>}
        </>
      )}
      <button
        type="button"
        data-wipe-cancel
        onClick={() => void emitView(WIPE_VIEW_ID, 'click', { target: WIPE_CANCEL_TARGET })}
      >
        {model.phase === 'confirm' ? 'Cancel' : 'Close'}
      </button>
    </div>
  );
}
