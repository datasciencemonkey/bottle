/** #573: a double-click shows and focuses the hub. */
import { isHubFocused, showHub, type HubWindowHandle } from './hub-window';

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

describe('isHubFocused (#145)', () => {
  it('asks the hub window for its OS focus', async () => {
    const asked: string[] = [];
    const find = (label: string) => {
      asked.push(label);
      return Promise.resolve({ isFocused: () => Promise.resolve(true) });
    };
    await expect(isHubFocused(find)).resolves.toBe(true);
    expect(asked).toEqual(['hub']);
    await expect(
      isHubFocused(() => Promise.resolve({ isFocused: () => Promise.resolve(false) })),
    ).resolves.toBe(false);
  });

  it('is false with no hub window, and when the call fails', async () => {
    await expect(isHubFocused(() => Promise.resolve(null))).resolves.toBe(false);
    await expect(
      isHubFocused(() => Promise.resolve({ isFocused: () => Promise.reject(new Error('fake')) })),
    ).resolves.toBe(false);
    await expect(isHubFocused(() => Promise.reject(new Error('fake')))).resolves.toBe(false);
  });
});
