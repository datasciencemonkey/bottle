/** #573: a double-click shows and focuses the hub. */
import { showHub, type HubWindowHandle } from './hub-window';

function fakeHub(fail?: keyof HubWindowHandle) {
  const steps: string[] = [];
  const step = (name: keyof HubWindowHandle) => () => {
    steps.push(name);
    return name === fail ? Promise.reject(new Error('fake')) : Promise.resolve();
  };
  const w: HubWindowHandle = {
    unminimize: step('unminimize'),
    show: step('show'),
    setFocus: step('setFocus'),
  };
  return { w, steps };
}

describe('showHub (#573)', () => {
  it('restores, shows and focuses the hub window, in that order', async () => {
    const h = fakeHub();
    const asked: string[] = [];
    await expect(
      showHub((label) => {
        asked.push(label);
        return Promise.resolve(h.w);
      }),
    ).resolves.toBe(true);
    expect(asked).toEqual(['hub']);
    expect(h.steps).toEqual(['unminimize', 'show', 'setFocus']);
  });

  it('is false, and never throws, with no hub or a failed step', async () => {
    await expect(showHub(() => Promise.resolve(null))).resolves.toBe(false);
    await expect(showHub(() => Promise.reject(new Error('fake')))).resolves.toBe(false);
    const h = fakeHub('show');
    await expect(showHub(() => Promise.resolve(h.w))).resolves.toBe(false);
    expect(h.steps).toEqual(['unminimize', 'show']);
  });
});
