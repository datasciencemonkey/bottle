/**
 * The Forget dialog (thin, MEM-32a, MEM-32c, MEM-32d, G-45b, AT-16; ADR 0081). The brain builds
 * the model (`src/brain/components/forget-flow.ts`). The confirm button sends `view_emit` with the
 * scope kind only; the brain runs Forget. The dialog links to the Genie One memory review
 * (MEM-32d) and, after the delete, mounts the "Delete in Genie One" notice (G-45b).
 * TODO(#87): open the memory review link in the system browser (the G-19a link opener).
 */
import type { ForgetDialogModel } from '../shared/forget-view';
import { FORGET_CONFIRM_TARGET, GENIE_ONE_MEMORY_REVIEW_TARGET } from '../shared/forget-view';
import { GenieOneDeleteNotice } from './GenieOneDeleteNotice';
import { emitView } from './view-ipc';

export function ForgetDialog({ model }: { model: ForgetDialogModel }) {
  const { memoryReview, result } = model;
  return (
    <div data-forget-dialog role="dialog" aria-label={model.title}>
      <h2>{model.title}</h2>
      {model.phase === 'confirm' && (
        <>
          <p>{model.text}</p>
          <button
            type="button"
            data-forget-confirm
            onClick={() =>
              void emitView(model.mogId, 'click', {
                target: FORGET_CONFIRM_TARGET,
                scope: model.scope,
              })
            }
          >
            Forget
          </button>
        </>
      )}
      {model.phase === 'done' && result !== undefined && (
        <>
          <p data-forget-result>
            Bottle deleted {result.local} {result.local === 1 ? 'row' : 'rows'} on this computer and{' '}
            {result.deletedConversations} Genie{' '}
            {result.deletedConversations === 1 ? 'conversation' : 'conversations'}.
          </p>
          {result.notice !== undefined && <GenieOneDeleteNotice notice={result.notice} />}
          {result.otherText !== undefined && <p data-forget-other>{result.otherText}</p>}
        </>
      )}
      <p>{memoryReview.text}</p>
      {memoryReview.url !== undefined && (
        <p>
          <button
            type="button"
            data-genie-one-memory-review
            onClick={() =>
              void emitView(model.mogId, 'click', { target: GENIE_ONE_MEMORY_REVIEW_TARGET })
            }
          >
            {memoryReview.label}
          </button>{' '}
          <span data-genie-one-memory-review-url>{memoryReview.url}</span>
        </p>
      )}
    </div>
  );
}
