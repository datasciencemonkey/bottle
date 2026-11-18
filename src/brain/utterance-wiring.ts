import type { Utterance as CapturedUtterance } from './capture-lifecycle';
import type { RouterInput } from './components/router-step';
import type { OwnedUtterance, RequestOwner } from './request-owner';
import type { UtteranceRoute, UtteranceRouteDeps } from './utterance-route';

export interface UtteranceHandlerDeps {
  owner: Pick<RequestOwner, 'begin' | 'release'>;
  routerInput(mogId: string): RouterInput | undefined;
  insertionOn(mogId: string): boolean;
  route: UtteranceRouteDeps;
  onAsk(r: Extract<UtteranceRoute, { route: 'preview' | 'chip' }>, owned: OwnedUtterance): void;
}

export function createUtteranceHandler(
  _deps: UtteranceHandlerDeps,
): (u: CapturedUtterance) => Promise<void> {
  return async () => undefined;
}
