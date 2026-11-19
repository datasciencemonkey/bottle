/**
 * #523: open a link from a Genie answer in the default browser.
 *
 * A view (the card or hub Conversations) never opens a URL: it sends
 * `view_emit` `click { target: 'link_open', url }`. The brain then
 * 1. finds the workspace of that window (`resolve`: the MOG's home profile for a card, the
 *    exchange's profile for hub Conversations, from the brain's own data, never from the click),
 * 2. checks the URL again with the same trust rule as the views (`trustedUrl`),
 * 3. asks Rust to open it (`link_open { profile, url }`), where Rust checks it a third time.
 * An untrusted or unresolved click does nothing and logs nothing (a URL is user content: SEC-16c).
 */
import type { ProfileName } from '../shared/interfaces';
import { asLinkOpenClick, trustedUrl } from '../shared/link-trust';
import { onViewInput, type BrainBus, type ViewInput } from './bus';

/** The workspace of a window: a profile name and its host (`https://host`). */
export interface LinkWorkspace {
  profile: ProfileName;
  host: string;
}

export interface LinkOpenDeps {
  /** The workspace that this click is about, or undefined if the brain does not know it. */
  resolve(input: ViewInput, requestId: string | undefined): Promise<LinkWorkspace | undefined>;
  /** `call('link_open', { profile, url })`. */
  open(args: { profile: ProfileName; url: string }): Promise<unknown>;
}

export function wireLinkOpen(bus: BrainBus, deps: LinkOpenDeps): () => void {
  return onViewInput(bus, (input) => {
    if (input.kind !== 'click') return;
    const click = asLinkOpenClick(input.payload);
    if (!click) return;
    void (async () => {
      let ws: LinkWorkspace | undefined;
      try {
        ws = await deps.resolve(input, click.requestId);
      } catch {
        ws = undefined;
      }
      if (!ws) return;
      const url = trustedUrl(click.url, { profileHost: ws.host, allowedAppUrls: [] });
      if (url === undefined) return;
      try {
        await deps.open({ profile: ws.profile, url });
      } catch {
        // Rust refused or the opener failed: nothing opens, and the view shows nothing more.
      }
    })();
  });
}
