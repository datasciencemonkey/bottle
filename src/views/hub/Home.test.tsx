// @vitest-environment jsdom
/** The Home landing under reduced motion: no fade, the stage is simply there. */
import { render } from '../test-tauri';
import { Home } from './Home';

const reduced = vi.hoisted(() => ({ value: true }));
vi.mock('motion/react', async (orig) => ({
  ...(await orig<typeof import('motion/react')>()),
  useReducedMotion: () => reduced.value,
}));

describe('Home landing motion', () => {
  it('[home] reduced motion shows the landing at once, with no fade', async () => {
    reduced.value = true;
    const { el } = await render(<Home hidden={false} />);
    const stage = el.querySelector<HTMLElement>('.hub-home-stage');
    expect(stage?.getAttribute('data-motion')).toBe('reduced');
    expect(stage?.style.opacity).toBe('1');
  });

  it('[home] full motion starts transparent and fades in', async () => {
    reduced.value = false;
    const { el } = await render(<Home hidden={false} />);
    const stage = el.querySelector<HTMLElement>('.hub-home-stage');
    expect(stage?.getAttribute('data-motion')).toBe('fade');
    expect(stage?.style.opacity).toBe('0');
  });
});
