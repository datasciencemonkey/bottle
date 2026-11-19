import {
  asMogTargetView,
  CARD_IDENTITY_READY,
  isCardIdentityReady,
  isViewReady,
  MOG_TARGET_EVENT,
  TARGET_TOAST_MS,
} from './mog-target';

const A = '00000000-0000-4000-8000-00000000000a';

describe('the view-local mog:target payload (#73, TODO(#439))', () => {
  it('[UX-28] [MB-06b] accepts only { mogId, name, target, toast }', () => {
    expect(MOG_TARGET_EVENT).toBe('mog:target');
    expect(TARGET_TOAST_MS).toBe(1200);
    const ok = { mogId: A, name: 'Sales', target: true, toast: false };
    expect(asMogTargetView(ok)).toEqual(ok);
    for (const bad of [
      null,
      'x',
      { ...ok, mogId: 1 },
      { ...ok, name: undefined },
      { ...ok, target: 'true' },
      { ...ok, toast: 1 },
    ]) {
      expect(asMogTargetView(bad)).toBeUndefined();
    }
  });

  it('[MB-06a] isViewReady reads only the view.ready action', () => {
    expect(isViewReady({ action: 'view.ready' })).toBe(true);
    for (const p of [undefined, null, {}, { action: 'notice.ready' }, 'view.ready']) {
      expect(isViewReady(p)).toBe(false);
    }
  });

  it('[ui-4] colorVariant is optional, and must be a string when present', () => {
    const ok = { mogId: A, name: 'Sales', target: true, toast: false };
    expect(asMogTargetView({ ...ok, colorVariant: 'coral' })).toEqual({
      ...ok,
      colorVariant: 'coral',
    });
    expect(asMogTargetView({ ...ok, colorVariant: 1 })).toBeUndefined();
  });

  it('[ui-4] isCardIdentityReady reads only the card.identity_ready action', () => {
    expect(CARD_IDENTITY_READY).toBe('card.identity_ready');
    expect(isCardIdentityReady({ action: CARD_IDENTITY_READY })).toBe(true);
    for (const p of [undefined, null, {}, { action: 'view.ready' }, 'card.identity_ready']) {
      expect(isCardIdentityReady(p)).toBe(false);
    }
  });

  it('[ui-4] home is optional, and must be a string when present', () => {
    const ok = { mogId: A, name: 'Sales', target: true, toast: false };
    expect(asMogTargetView({ ...ok, home: 'sales-ws' })).toEqual({ ...ok, home: 'sales-ws' });
    expect(asMogTargetView({ ...ok, home: 7 })).toBeUndefined();
  });
});
