import { asMogTargetView, isViewReady, MOG_TARGET_EVENT, TARGET_TOAST_MS } from './mog-target';

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
});
