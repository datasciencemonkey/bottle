import {
  CARD_WINDOW_MAX_HEIGHT,
  CARD_WINDOW_MIN_HEIGHT,
  cardWindowHeight,
  createWindowFit,
  type CardWindow,
} from './fit-window';

function fakeWindow(place = { x: 10, top: 100, screenBottom: 900 }, fail = false) {
  const log: unknown[][] = [];
  const w: CardWindow = {
    setHeight: (h) => {
      log.push(['height', h]);
      return fail ? Promise.reject(new Error('fake')) : Promise.resolve();
    },
    place: () => Promise.resolve(place),
    moveTo: (x, top) => {
      log.push(['move', x, top]);
      return Promise.resolve();
    },
  };
  return { w, log };
}

describe('card window fit (#573)', () => {
  it('keeps the height between one pop-up line and the cap', () => {
    expect(cardWindowHeight(20)).toBe(CARD_WINDOW_MIN_HEIGHT);
    expect(cardWindowHeight(180.2)).toBe(181);
    expect(cardWindowHeight(5000)).toBe(CARD_WINDOW_MAX_HEIGHT);
    expect(cardWindowHeight(Number.NaN)).toBe(CARD_WINDOW_MIN_HEIGHT);
  });

  it('sets each new height once and does not move a card that fits the screen', async () => {
    const f = fakeWindow();
    const fit = createWindowFit(() => f.w);
    await fit(200);
    await fit(199.6);
    await fit(64);
    expect(f.log).toEqual([
      ['height', 200],
      ['height', 64],
    ]);
  });

  it('fits again when the window lost its height (R11 shows a hidden card at one line)', async () => {
    const f = fakeWindow();
    const fit = createWindowFit(() => f.w);
    await fit(200);
    await fit(200, 200);
    await fit(200, 64);
    expect(f.log).toEqual([
      ['height', 200],
      ['height', 200],
    ]);
  });

  it('moves a card up that would go past the bottom of the screen', async () => {
    const f = fakeWindow({ x: 10, top: 800, screenBottom: 900 });
    await createWindowFit(() => f.w)(300);
    expect(f.log).toEqual([
      ['height', 300],
      ['move', 10, 600],
    ]);
  });

  it('tries again after a failed resize', async () => {
    const f = fakeWindow(undefined, true);
    const fit = createWindowFit(() => f.w);
    await fit(200);
    await fit(200);
    expect(f.log).toEqual([
      ['height', 200],
      ['height', 200],
    ]);
  });
});
