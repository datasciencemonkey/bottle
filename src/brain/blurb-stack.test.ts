import {
  BlurbStack,
  DEFAULT_AUTO_HIDE_SEC,
  DEFAULT_MAX_BLURBS,
  decodeCardInput,
  type StackView,
} from './blurb-stack';

const S = 1000;
const ids = (v: StackView) => v.blurbs.map((b) => b.id);

/** Add blurbs b1..bn, one second apart, so b<n> is the newest. */
function stackWith(n: number, opts?: ConstructorParameters<typeof BlurbStack>[0]): BlurbStack {
  const s = new BlurbStack(opts);
  for (let i = 1; i <= n; i++) s.add({ id: `b${i}`, text: `answer ${i}` }, i * S);
  return s;
}

describe('BlurbStack', () => {
  it('[UX-11a] uses the Appendix F defaults: 3 blurbs and 60 s', () => {
    expect(DEFAULT_MAX_BLURBS).toBe(3);
    expect(DEFAULT_AUTO_HIDE_SEC).toBe(60);
    expect(new BlurbStack().settings).toEqual({ maxBlurbs: 3, autoHideSec: 60 });
  });

  it('[UX-11a] [AT-09] shows 3 blurbs or less, newest on top', () => {
    const s = stackWith(2);
    expect(ids(s.view())).toEqual(['b2', 'b1']);
    s.add({ id: 'b3', text: 'answer 3' }, 3 * S);
    expect(ids(s.view())).toEqual(['b3', 'b2', 'b1']);
    expect(s.view().historyCount).toBe(0);
  });

  it('[UX-11a] [AT-09] older blurbs go to History with a count', () => {
    const s = stackWith(5);
    expect(ids(s.view())).toEqual(['b5', 'b4', 'b3']);
    expect(s.view().historyCount).toBe(2);
    expect(s.history()).toEqual(['b2', 'b1']);
  });

  it('[UX-11a] a maxBlurbs setting above 3 is clamped to 3; a bad value falls back', () => {
    expect(new BlurbStack({ maxBlurbs: 10 }).settings.maxBlurbs).toBe(3);
    expect(new BlurbStack({ maxBlurbs: 2 }).settings.maxBlurbs).toBe(2);
    expect(new BlurbStack({ maxBlurbs: 0 }).settings.maxBlurbs).toBe(1);
    expect(new BlurbStack({ maxBlurbs: Number.NaN }).settings.maxBlurbs).toBe(3);
    expect(new BlurbStack({ autoHideSec: -5 }).settings.autoHideSec).toBe(60);
    expect(ids(stackWith(5, { maxBlurbs: 10 }).view())).toHaveLength(3);
  });

  it('[UX-11a] the same blurb ID again updates the text and takes no new slot', () => {
    const s = stackWith(3);
    s.add({ id: 'b2', text: 'answer 2, final' }, 10 * S);
    expect(ids(s.view())).toEqual(['b3', 'b2', 'b1']);
    expect(s.view().blurbs[1]?.text).toBe('answer 2, final');
  });

  it('[UX-11b] [AT-09] a 3-part split takes one slot', () => {
    const s = stackWith(2);
    for (const p of ['p1', 'p2', 'p3'])
      s.add({ id: p, text: `part ${p}`, splitGroupId: 'g1' }, 5 * S);
    const v = s.view();
    expect(ids(v)).toEqual(['g1', 'b2', 'b1']);
    expect(v.historyCount).toBe(0);
    expect(v.blurbs[0]?.parts?.map((p) => p.id)).toEqual(['p1', 'p2', 'p3']);
  });

  it('[UX-11b] a split part that comes later joins its slot and does not push a blurb out', () => {
    const s = new BlurbStack();
    s.add({ id: 'p1', text: 'part 1', splitGroupId: 'g1' }, 1 * S);
    s.add({ id: 'b1', text: 'answer 1' }, 2 * S);
    s.add({ id: 'b2', text: 'answer 2' }, 3 * S);
    s.add({ id: 'p2', text: 'part 2', splitGroupId: 'g1' }, 4 * S);
    expect(ids(s.view())).toEqual(['b2', 'b1', 'g1']);
    expect(s.view().blurbs[2]?.parts).toHaveLength(2);
  });

  it('[UX-11b] a split counts as one History entry when it is pushed out', () => {
    const s = new BlurbStack();
    for (const p of ['p1', 'p2', 'p3']) s.add({ id: p, text: p, splitGroupId: 'g1' }, S);
    for (let i = 1; i <= 3; i++) s.add({ id: `b${i}`, text: `a${i}` }, (1 + i) * S);
    expect(s.view().historyCount).toBe(1);
    expect(s.history()).toEqual(['g1']);
  });

  it('[UX-11c] [AT-09] a pinned blurb stays after 3 new blurbs', () => {
    const s = stackWith(1);
    expect(s.pin('b1')).toBe(true);
    for (let i = 2; i <= 4; i++) s.add({ id: `b${i}`, text: `answer ${i}` }, i * S);
    const v = s.view();
    expect(ids(v)).toEqual(['b4', 'b3', 'b1']);
    expect(v.blurbs[2]?.pinned).toBe(true);
    expect(v.historyCount).toBe(1);
    expect(s.history()).toEqual(['b2']);
  });

  it('[UX-11c] [UX-11a] pins never push the card above 3, and a slot stays free for the newest', () => {
    const s = stackWith(3);
    expect(s.pin('b1')).toBe(true);
    expect(s.pin('b2')).toBe(true);
    expect(s.pin('b3')).toBe(false); // the last slot is for the newest answer
    s.add({ id: 'b4', text: 'answer 4' }, 4 * S);
    expect(ids(s.view())).toEqual(['b4', 'b2', 'b1']);
    expect(s.view().blurbs.filter((b) => b.pinned).map((b) => b.id)).toEqual(['b2', 'b1']);
  });

  it('[UX-11c] an unpinned blurb is the next one to go to History', () => {
    const s = stackWith(1);
    s.pin('b1');
    s.add({ id: 'b2', text: 'a2' }, 2 * S);
    s.add({ id: 'b3', text: 'a3' }, 3 * S);
    expect(s.unpin('b1')).toBe(true);
    s.add({ id: 'b4', text: 'a4' }, 4 * S);
    expect(ids(s.view())).toEqual(['b4', 'b3', 'b2']);
    expect(s.history()).toEqual(['b1']);
  });

  it('[UX-11c] pin and unpin of an unknown or History blurb do nothing', () => {
    const s = stackWith(4);
    expect(s.pin('nope')).toBe(false);
    expect(s.pin('b1')).toBe(false);
    expect(s.unpin('b1')).toBe(false);
    expect(s.view().historyCount).toBe(1);
  });

  it('[UX-12] [AT-09] a completed blurb with no hover for 60 s collapses into History', () => {
    const s = stackWith(2);
    s.complete('b1', 10 * S);
    expect(s.nextDeadline()).toBe(70 * S);
    expect(s.tick(70 * S - 1)).toEqual([]);
    expect(ids(s.view())).toEqual(['b2', 'b1']);
    expect(s.tick(70 * S)).toEqual(['b1']);
    expect(ids(s.view())).toEqual(['b2']);
    expect(s.view().historyCount).toBe(1);
    expect(s.history()).toEqual(['b1']);
  });

  it('[UX-12] a blurb that is not complete never collapses', () => {
    const s = stackWith(1);
    expect(s.nextDeadline()).toBeUndefined();
    expect(s.tick(3600 * S)).toEqual([]);
    expect(ids(s.view())).toEqual(['b1']);
  });

  it('[UX-12] a hover holds the blurb, and the 60 s start again when the hover ends', () => {
    const s = stackWith(1);
    s.complete('b1', 0);
    s.hover('b1', 'pointer', true, 30 * S);
    expect(s.nextDeadline()).toBeUndefined();
    expect(s.tick(120 * S)).toEqual([]);
    s.hover('b1', 'pointer', false, 120 * S);
    expect(s.nextDeadline()).toBe(180 * S);
    expect(s.tick(180 * S)).toEqual(['b1']);
  });

  it('[UX-12] pointer and keyboard focus are tracked apart, so one ending does not end the other', () => {
    const s = stackWith(1);
    s.complete('b1', 0);
    s.hover('b1', 'pointer', true, S);
    s.hover('b1', 'focus', true, S);
    s.hover('b1', 'pointer', false, 2 * S);
    expect(s.nextDeadline()).toBeUndefined();
    s.hover('b1', 'focus', false, 5 * S);
    expect(s.nextDeadline()).toBe(65 * S);
  });

  it('[UX-12] [UX-11c] a pinned blurb does not collapse; after unpin the 60 s start again', () => {
    const s = stackWith(1);
    s.complete('b1', 0);
    s.pin('b1');
    expect(s.nextDeadline()).toBeUndefined();
    expect(s.tick(600 * S)).toEqual([]);
    s.unpin('b1');
    s.tick(600 * S);
    expect(s.nextDeadline()).toBe(660 * S);
  });

  it('[UX-12] [UX-11b] a split collapses only when every part is complete', () => {
    const s = new BlurbStack();
    s.add({ id: 'p1', text: 'p1', splitGroupId: 'g1' }, 0);
    s.add({ id: 'p2', text: 'p2', splitGroupId: 'g1' }, 0);
    s.complete('p1', 0);
    expect(s.nextDeadline()).toBeUndefined();
    expect(s.view().blurbs[0]?.completed).toBe(false);
    s.complete('p2', 5 * S);
    expect(s.view().blurbs[0]?.completed).toBe(true);
    expect(s.tick(65 * S)).toEqual(['g1']);
  });

  it('[UX-12] the autoHideSec setting changes the collapse time', () => {
    const s = stackWith(1, { autoHideSec: 10 });
    s.complete('b1', 0);
    expect(s.nextDeadline()).toBe(10 * S);
  });

  it('[UX-12] a blurb added as complete starts its 60 s at once', () => {
    const s = new BlurbStack();
    s.add({ id: 'b1', text: 'a1', completed: true }, 5 * S);
    expect(s.nextDeadline()).toBe(65 * S);
  });
});

describe('decodeCardInput', () => {
  it('[UX-12] decodes hover input of the card view', () => {
    expect(decodeCardInput('hover', { blurbId: 'b1', on: true, via: 'pointer' })).toEqual({
      t: 'hover',
      entryId: 'b1',
      via: 'pointer',
      on: true,
    });
    expect(decodeCardInput('hover', { blurbId: 'b1', on: false, via: 'focus' })).toEqual({
      t: 'hover',
      entryId: 'b1',
      via: 'focus',
      on: false,
    });
  });

  it('[UX-11c] [UX-11a] decodes pin and History clicks', () => {
    expect(decodeCardInput('click', { target: 'pin', blurbId: 'b1' })).toEqual({
      t: 'toggle_pin',
      entryId: 'b1',
    });
    expect(decodeCardInput('click', { target: 'history' })).toEqual({ t: 'open_history' });
  });

  it('[UX-11a] drops malformed or unrelated input', () => {
    expect(decodeCardInput('hover', undefined)).toBeUndefined();
    expect(decodeCardInput('hover', { blurbId: 1, on: true, via: 'pointer' })).toBeUndefined();
    expect(decodeCardInput('hover', { blurbId: 'b1', on: 'yes', via: 'pointer' })).toBeUndefined();
    expect(decodeCardInput('hover', { blurbId: 'b1', on: true, via: 'eye' })).toBeUndefined();
    expect(decodeCardInput('click', { target: 'pin' })).toBeUndefined();
    expect(decodeCardInput('click', { target: 'other' })).toBeUndefined();
    expect(decodeCardInput('expand', { target: 'history' })).toBeUndefined();
  });
});
