// @vitest-environment jsdom
import {
  CARD_WINDOW_MAX_HEIGHT,
  CARD_WINDOW_MIN_HEIGHT,
  CARD_WINDOW_WIDTH,
  applyCardSide,
  cardWindowHeight,
  cardWindowWidth,
  createWindowFit,
  type CardPlace,
  type CardWindow,
} from './fit-window';
import type { Rect } from './card-placement';

const SCREEN: Rect = { x: 0, y: 0, w: 1440, h: 900 };

function fakeWindow(place: CardPlace | undefined, fail = false) {
  const log: { r: Rect; from: Rect }[] = [];
  const w: CardWindow = {
    setBounds: (r, from) => {
      log.push({ r, from });
      return fail ? Promise.reject(new Error('fake')) : Promise.resolve();
    },
    place: () => Promise.resolve(place),
  };
  return { w, log };
}

const rightCard: CardPlace = {
  card: { x: 745, y: 342, w: 340, h: 64 },
  side: 'right',
  screen: SCREEN,
};
const leftCard: CardPlace = {
  card: { x: 930, y: 342, w: 340, h: 64 },
  side: 'left',
  screen: SCREEN,
};

describe('card window fit (#573, lamp-alive)', () => {
  it('keeps the height between one pop-up line and the cap', () => {
    expect(cardWindowHeight(20)).toBe(CARD_WINDOW_MIN_HEIGHT);
    expect(cardWindowHeight(180.2)).toBe(181);
    expect(cardWindowHeight(5000)).toBe(CARD_WINDOW_MAX_HEIGHT);
    expect(cardWindowHeight(Number.NaN)).toBe(CARD_WINDOW_MIN_HEIGHT);
  });

  it('sizes the window to the pill plus the two pads, with no minimum width', () => {
    expect(cardWindowWidth(undefined)).toBe(CARD_WINDOW_WIDTH);
    expect(cardWindowWidth(Number.NaN)).toBe(CARD_WINDOW_WIDTH);
    // 10 px facing the MOG and 12 px on the far side.
    // The text width is rounded up and has 1 px of slack, so a fraction never cuts a letter.
    expect(cardWindowWidth(210.2)).toBe(211 + 1 + 22);
    expect(cardWindowWidth(210)).toBe(210 + 1 + 22);
    expect(cardWindowWidth(100)).toBe(123);
    expect(cardWindowWidth(900)).toBe(CARD_WINDOW_WIDTH);
  });

  it('sets each new size once, and fits again when the pill widens (the timer comes in)', async () => {
    const f = fakeWindow(rightCard);
    let measured: number | undefined = 200;
    const fit = createWindowFit(() => f.w, { measureWidth: () => measured });
    await fit(64);
    await fit(63.6);
    measured = 238; // the timer shows after 3 s
    await fit(64);
    measured = undefined;
    await fit(200);
    expect(f.log.map((l) => [l.r.w, l.r.h])).toEqual([
      [223, 64],
      [261, 64],
      [340, 200],
    ]);
  });

  it('fits again when the window lost its height (R11 shows a hidden card at one line)', async () => {
    const f = fakeWindow(rightCard);
    const fit = createWindowFit(() => f.w, { measureWidth: () => undefined });
    await fit(200);
    await fit(200, 200);
    await fit(200, 64);
    expect(f.log).toHaveLength(2);
  });

  it('keeps the edge facing the MOG: right grows right, left grows left, whatever the width', async () => {
    const sides: string[] = [];
    for (const width of [150, 200, 328, undefined]) {
      const r = fakeWindow(rightCard);
      await createWindowFit(() => r.w, { measureWidth: () => width, onSide: (s) => sides.push(s) })(
        64,
      );
      expect(r.log[0]?.r.x, `right ${width}`).toBe(745);
      const l = fakeWindow(leftCard);
      await createWindowFit(() => l.w, { measureWidth: () => width })(64);
      const b = l.log[0]?.r as Rect;
      expect(b.x + b.w, `left ${width}`).toBe(1270);
    }
    expect(new Set(sides)).toEqual(new Set(['right']));
  });

  it('tells the view the side the host chose', async () => {
    const sides: string[] = [];
    const f = fakeWindow(leftCard);
    await createWindowFit(() => f.w, { measureWidth: () => 200, onSide: (s) => sides.push(s) })(64);
    expect(sides).toEqual(['left']);
  });

  it('moves a card up that would go past the bottom of the screen', async () => {
    const f = fakeWindow({ ...rightCard, card: { x: 745, y: 800, w: 340, h: 64 } });
    await createWindowFit(() => f.w, { measureWidth: () => undefined })(300);
    const r = f.log[0]?.r as Rect;
    expect(r.y + r.h).toBe(900);
  });

  it('gives the old rectangle too, so a window that grows moves first and one that shrinks shrinks first', async () => {
    const f = fakeWindow(leftCard);
    await createWindowFit(() => f.w, { measureWidth: () => 200 })(64);
    expect(f.log[0]?.from).toEqual(leftCard.card);
  });

  it('tries again after a failed resize', async () => {
    const f = fakeWindow(rightCard, true);
    const fit = createWindowFit(() => f.w, { measureWidth: () => undefined });
    await fit(200);
    await fit(200);
    expect(f.log).toHaveLength(2);
  });
});

describe('lamp-alive: the pill is measured with its fraction', () => {
  it('reads the layout width as a fraction, so a text of 209.56 px gets a window of 210 + 1 + 22', async () => {
    const { measureCompactWidth } = await import('./fit-window');
    document.body.innerHTML = '<div class="popup" style="width: 209.56px"></div>';
    const w = measureCompactWidth(document);
    expect(w).toBeCloseTo(209.56, 2);
    expect(cardWindowWidth(w)).toBe(210 + 1 + 22);
    // The max-width of the window is not left on the element.
    expect((document.querySelector('.popup') as HTMLElement).style.maxWidth).toBe('');
    document.body.innerHTML = '';
  });

  it('[AC-1.13] takes the wider of a pill and a pop-up that show together', async () => {
    const { measureCompactWidth } = await import('./fit-window');
    document.body.innerHTML =
      '<div class="genie genie-working" style="width: 190px"></div>' +
      '<div class="popup" style="width: 260.5px"></div>';
    const w = measureCompactWidth(document);
    expect(w).toBeCloseTo(260.5, 2);
    expect(cardWindowWidth(w)).toBe(261 + 1 + 22);
    for (const el of document.querySelectorAll<HTMLElement>('.popup, .genie-working'))
      expect(el.style.maxWidth).toBe('');
    document.body.innerHTML = '';
  });

  it('[AC-1.13] the window takes the height of both rows, so no clear gap stays under them', async () => {
    const f = fakeWindow(rightCard);
    const fit = createWindowFit(() => f.w, { measureWidth: () => 260.5 });
    // The pill (38 px), a gap (8 px) and the pop-up (40 px) with the window padding (24 px).
    await fit(38 + 8 + 40 + 24);
    expect(f.log).toHaveLength(1);
    expect(f.log[0]?.r.h).toBe(110);
    expect(f.log[0]?.r.w).toBe(261 + 1 + 22);
    // The edge that faces the MOG stays where it was.
    expect(f.log[0]?.r.x).toBe(rightCard.card.x);
  });

  it('is undefined for the answer card, which keeps the full width', async () => {
    const { measureCompactWidth } = await import('./fit-window');
    document.body.innerHTML = '<div class="genie genie-answer"></div>';
    expect(measureCompactWidth(document)).toBeUndefined();
    document.body.innerHTML = '';
  });
});

describe('lamp-alive: a timer tick does not force a layout read', () => {
  const text = (parent: string, old: string, now: string) => {
    document.body.innerHTML = parent;
    const t = document.body.querySelector('span')?.firstChild as Text;
    t.data = now;
    return { type: 'characterData' as const, target: t, oldValue: old };
  };

  it('skips a tick that keeps its digits, and fits when the timer gets a digit', async () => {
    const { widthMayChangeIn } = await import('./fit-window');
    const timer = '<span data-genie-timer>0:04</span>';
    expect(widthMayChangeIn([text(timer, '0:03', '0:04')])).toBe(false);
    expect(widthMayChangeIn([text(timer, '0:59', '1:00')])).toBe(false);
    expect(widthMayChangeIn([text(timer, '9:59', '10:00')])).toBe(true);
  });

  it('fits for a new node (the timer appears) and for any other text change', async () => {
    const { widthMayChangeIn } = await import('./fit-window');
    const node = document.createElement('span');
    expect(widthMayChangeIn([{ type: 'childList', target: node, oldValue: null }])).toBe(true);
    expect(widthMayChangeIn([text('<span>Asking Genie</span>', 'Asking', 'Asking Genie')])).toBe(
      true,
    );
    expect(widthMayChangeIn([])).toBe(false);
    document.body.innerHTML = '';
  });
});

describe('lamp-alive: the side mark', () => {
  it('marks the page with the side, for the tail in card.css', () => {
    applyCardSide('left');
    expect(document.documentElement.dataset['side']).toBe('left');
    applyCardSide('right');
    expect(document.documentElement.dataset['side']).toBe('right');
    delete document.documentElement.dataset['side'];
  });
});
