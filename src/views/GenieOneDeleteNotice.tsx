/**
 * The "Delete in Genie One" notice (thin, G-45b, AT-16). It shows the disclosure from the brain
 * and one link for each Genie One conversation that Bottle did not delete. A click goes to the
 * brain with `view_emit` and the conversation ID only. The brain owns the link. The URL also
 * shows as text, so the user can open it in a browser.
 * TODO(#87): open the safe thread link in the system browser (the G-19a link opener).
 */
import type { GenieOneDeleteNoticeModel } from '../shared/genie-one-delete-view';
import { GENIE_ONE_DELETE_TARGET } from '../shared/genie-one-delete-view';
import { emitView } from './view-ipc';

export function GenieOneDeleteNotice({ notice }: { notice: GenieOneDeleteNoticeModel }) {
  return (
    <div data-genie-one-delete role="status">
      <p>{notice.text}</p>
      <ul>
        {notice.links.map((l) => (
          <li key={l.conversationId}>
            <button
              type="button"
              data-genie-one-delete-link
              onClick={() =>
                void emitView(notice.mogId, 'click', {
                  target: GENIE_ONE_DELETE_TARGET,
                  conversationId: l.conversationId,
                })
              }
            >
              {notice.label}
            </button>{' '}
            <span data-genie-one-delete-url>{l.url}</span>
          </li>
        ))}
      </ul>
      {notice.noLink > 0 && notice.noLinkText !== undefined && (
        <p data-genie-one-delete-nolink>{notice.noLinkText}</p>
      )}
    </div>
  );
}
