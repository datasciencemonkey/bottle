// @vitest-environment jsdom
/**
 * The Home landing: its structure and accessibility, the reduced-motion final frame, the
 * entrance gate and the idle pause (the motion itself is CSS and Motion, checked in a browser).
 */
import { act } from 'react';
import { render } from '../test-tauri';
import { Home } from './Home';

const reduced = vi.hoisted(() => ({ value: true }));
vi.mock('motion/react', async (orig) => ({
  ...(await orig<typeof import('motion/react')>()),
  useReducedMotion: () => reduced.value,
}));

const section = (el: HTMLElement) => el.querySelector<HTMLElement>('[data-hub-view="home"]');

function setVisibility(state: 'visible' | 'hidden'): void {
  Object.defineProperty(document, 'visibilityState', { value: state, configurable: true });
  act(() => {
    document.dispatchEvent(new Event('visibilitychange'));
  });
}

afterEach(() => setVisibility('visible'));

describe('Home landing', () => {
  it('[home] adds no h1 of its own: the hub header keeps the one h1, the wordmark reads "Bottle"', async () => {
    reduced.value = true;
    const { el } = await render(<Home hidden={false} />);
    expect(el.querySelectorAll('h1')).toHaveLength(0);
    const wordmark = el.querySelector<HTMLElement>('[data-home-wordmark]');
    expect(wordmark?.tagName).toBe('P');
    expect(wordmark?.querySelector('.sr-only')?.textContent).toBe('Bottle');
    // The split letters are for the eye only, so a screen reader does not spell the word.
    expect(wordmark?.querySelector('.home-letters')?.getAttribute('aria-hidden')).toBe('true');
    expect(section(el)?.getAttribute('aria-label')).toBe('Home');
  });

  it('[home] the decorative layers are hidden from assistive tech, and nothing takes focus', async () => {
    reduced.value = true;
    const { el } = await render(<Home hidden={false} />);
    // The art is one image to a screen reader; its layers (rays, lamp, spark) are hidden.
    const art = el.querySelector('[data-home-art]');
    expect(art?.getAttribute('role')).toBe('img');
    expect(art?.getAttribute('aria-label')).toContain('peeking out of its lamp');
    expect(art?.querySelectorAll('.home-depth')).toHaveLength(3);
    for (const layer of art?.querySelectorAll('.home-depth') ?? []) {
      expect(layer.getAttribute('aria-hidden')).toBe('true');
    }
    expect(el.querySelector('[data-home-rule]')?.getAttribute('aria-hidden')).toBe('true');
    for (const img of el.querySelectorAll('img')) {
      expect(img.closest('[aria-hidden="true"]')).not.toBeNull();
      expect(img.getAttribute('alt')).toBe('');
    }
    for (const kbd of el.querySelectorAll('kbd')) {
      expect(kbd.closest('[aria-hidden="true"]')).not.toBeNull();
    }
    expect(el.querySelector('a, button, input, [tabindex]')).toBeNull();
  });

  it('[home] the key hints say the three shortcuts in words', async () => {
    reduced.value = true;
    const { el } = await render(<Home hidden={false} />);
    const said = [...el.querySelectorAll('.home-hint .sr-only')].map((n) => n.textContent);
    expect(said).toEqual([
      'Hold Right Option to dictate.',
      'Hold Right Option and tap G to ask Genie.',
      'Hold Right Option and tap R to start or stop a call recording.',
    ]);
    expect(el.querySelector('[data-home-punchline]')?.textContent).toBe('Your Genie, ready.');
  });

  it('[home] reduced motion renders the final frame at once: no entrance, no idle loops', async () => {
    reduced.value = true;
    const { el } = await render(<Home hidden={false} />);
    const home = section(el);
    expect(home?.getAttribute('data-motion')).toBe('reduced');
    // `idle` lifts the CSS entrance gate; `reduced` keeps every loop off (home.css).
    expect(home?.getAttribute('data-phase')).toBe('idle');
    // No element was left mid-entrance by an inline value.
    for (const n of el.querySelectorAll<HTMLElement>('[data-home-letter], [data-home-hint]')) {
      expect(n.style.opacity).toBe('');
      expect(n.style.transform).toBe('');
    }
  });

  it('[home] full motion starts in the entrance phase, with the gate on', async () => {
    reduced.value = false;
    const { el } = await render(<Home hidden={false} />);
    const home = section(el);
    expect(home?.getAttribute('data-motion')).toBe('full');
    expect(home?.getAttribute('data-phase')).toBe('intro');
  });

  it('[home] the idle life pauses while Home is hidden or the window is not visible', async () => {
    reduced.value = false;
    const { el } = await render(<Home hidden={false} />);
    expect(section(el)?.hasAttribute('data-paused')).toBe(false);
    setVisibility('hidden');
    expect(section(el)?.hasAttribute('data-paused')).toBe(true);
    setVisibility('visible');
    expect(section(el)?.hasAttribute('data-paused')).toBe(false);

    const { el: away } = await render(<Home hidden />);
    expect(section(away)?.hasAttribute('data-paused')).toBe(true);
    expect(section(away)?.hidden).toBe(true);
  });
});
