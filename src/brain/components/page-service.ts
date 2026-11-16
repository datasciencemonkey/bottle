/**
 * T14 PageService (4.3, hidden TS brain window). Implements the 4.7 `PageService`.
 *
 * Responsibilities: UC Page review, `POST /api/2.0/knowledge/pages` with `state:"DRAFT"`,
 * domain ID check, `request_id`, content-free audit (B3).
 */
import type { PageDraft, PageService } from '../../shared/interfaces';
import { NotImplementedError } from '../errors';

const C = 'PageService';

export class PageServiceStub implements PageService {
  async propose(_d: PageDraft): Promise<{ reviewId: string }> {
    throw new NotImplementedError(C, 'propose', 204); // TODO(#204)
  }
  /** Only after user approval of the review card. */
  async createDraft(_reviewId: string): Promise<{ pageId: string; state: 'DRAFT' }> {
    throw new NotImplementedError(C, 'createDraft', 203); // TODO(#203)
  }
}

export function createPageService(): PageService {
  return new PageServiceStub();
}
