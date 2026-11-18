/**
 * The hub notices of unshown dictations (#498, #509, ADR 0091; MOG-05b, MOG-06d). When a MOG card
 * cannot show, the brain sends the dictated text to the hub only, on the hub's private Channel.
 * The hub shows the text with Copy, so no dictated text is dropped. The hub only copies or
 * dismisses; it never sends the text anywhere and never logs it.
 *
 * TODO(#213): Copy writes the clipboard with the concealed and transient flags. No clipboard
 * command is on main yet, so the text is selectable and the button only shows.
 */
import { useEffect, useState } from 'react';
import type { HubTextNotice } from '../../shared/ipc';
import { HUB_SCOPE } from '../../shared/hub-notice';
import { sendViewReady, subscribeHubNotices } from '../view-ipc';

interface Shown extends HubTextNotice {
  key: number;
}

export function DictationNotices() {
  const [notices, setNotices] = useState<Shown[]>([]);
  useEffect(() => {
    let gone = false;
    let n = 0;
    void subscribeHubNotices((notice) => {
      if (!gone) setNotices((all) => [...all, { ...notice, key: ++n }]);
    }).then((ok) => {
      // #507: `ready` after the Channel is set, so the brain sends the notices it kept.
      if (ok && !gone) sendViewReady(HUB_SCOPE);
    });
    return () => {
      gone = true;
    };
  }, []);
  if (notices.length === 0) return null;
  return (
    <section aria-label="Dictations to copy">
      {notices.map((d) => (
        <div key={d.key} data-notice="dictation_unshown" role="status">
          <p>Bottle could not show this dictation on its card:</p>
          <p data-text style={{ userSelect: 'text' }}>
            {d.text}
          </p>
          <button type="button" data-action="copy">
            Copy
          </button>
          <button
            type="button"
            data-action="dismiss"
            onClick={() => setNotices((all) => all.filter((x) => x.key !== d.key))}
          >
            Dismiss
          </button>
        </div>
      ))}
    </section>
  );
}
