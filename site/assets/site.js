// Bottle landing page. Small vanilla script: scales the product scene, plays its loop,
// and keeps it still for reduced motion. The page reads fine without it.
(() => {
  'use strict';

  const reduce = window.matchMedia('(prefers-reduced-motion: reduce)');

  // Sticky nav hairline once the page scrolls.
  const nav = document.querySelector('.nav');
  const onScroll = () => nav && nav.classList.toggle('is-stuck', window.scrollY > 8);
  window.addEventListener('scroll', onScroll, { passive: true });
  onScroll();

  // The genie cheers while the pointer or focus is on the main download button.
  const lamp = document.querySelector('[data-lamp]');
  document.querySelectorAll('[data-cheer]').forEach((el) => {
    const on = () => lamp && lamp.setAttribute('data-cheer', '');
    const off = () => lamp && lamp.removeAttribute('data-cheer');
    el.addEventListener('pointerenter', on);
    el.addEventListener('pointerleave', off);
    el.addEventListener('focus', on);
    el.addEventListener('blur', off);
  });

  // ------------------------------------------------------------ the product scene
  const frame = document.querySelector('[data-scene-frame]');
  const scene = document.querySelector('[data-scene]');
  if (!frame || !scene) return;

  const WIDE = { w: 1120, h: 660 };
  const COMPACT = { w: 358, h: 620 };

  const fit = () => {
    const compact = frame.clientWidth < 600;
    const size = compact ? COMPACT : WIDE;
    scene.classList.toggle('compact', compact);
    frame.classList.toggle('is-compact', compact);
    scene.style.setProperty('--k', String(frame.clientWidth / size.w));
  };
  new ResizeObserver(fit).observe(frame);
  fit();

  // Split the question into words so it can arrive as it is spoken.
  const q = scene.querySelector('[data-question]');
  const words = q.textContent.trim().split(/\s+/);
  q.textContent = '';
  const spans = words.map((word, i) => {
    const s = document.createElement('span');
    s.className = 'w';
    s.textContent = word;
    q.append(s);
    if (i < words.length - 1) q.append(' ');
    return s;
  });

  const hud = scene.querySelector('.hud');
  const keys = (set) => hud.setAttribute('data-keys', set);
  const key = (name) => scene.querySelector(`[data-key="${name}"]`);
  const setStep = (step) => scene.setAttribute('data-step', step);
  const words$ = (n) => spans.forEach((s, i) => s.classList.toggle('on', i < n));

  const poster = () => {
    clear();
    words$(spans.length);
    scene.removeAttribute('data-hl');
    scene.querySelectorAll('.is-down').forEach((k) => k.classList.remove('is-down'));
    scene.setAttribute('data-poster', '');
    keys('opt g');
    setStep('answer');
  };

  // One loop, about 15 seconds. Each beat has a cause on screen.
  const WORD_MS = 210;
  const beats = [
    [
      0,
      () => {
        scene.removeAttribute('data-poster');
        scene.removeAttribute('data-hl');
        words$(0);
        setStep('idle');
      },
    ],
    [
      900,
      () => {
        keys('opt');
        key('opt').classList.add('is-down');
        setStep('listen');
      },
    ],
    [
      1700,
      () => {
        keys('opt g');
        key('g').classList.add('is-down');
        setStep('ask');
      },
    ],
    [1960, () => key('g').classList.remove('is-down')],
    ...spans.map((_, i) => [2000 + i * WORD_MS, () => words$(i + 1)]),
    [
      2000 + spans.length * WORD_MS + 500,
      () => {
        key('opt').classList.remove('is-down');
        setStep('wait');
      },
    ],
    [7600, () => setStep('answer')],
    [10000, () => scene.setAttribute('data-hl', '1')],
    [11000, () => scene.setAttribute('data-hl', '2')],
    [12000, () => scene.removeAttribute('data-hl')],
    [
      12900,
      () => {
        keys('esc');
        key('esc').classList.add('is-down');
        setStep('close');
      },
    ],
    [13150, () => key('esc').classList.remove('is-down')],
    [13900, () => setStep('idle')],
  ];
  const LOOP_MS = 14800;

  let timers = [];
  let playing = false;
  let userPaused = false;
  let visible = true;

  function clear() {
    timers.forEach(clearTimeout);
    timers = [];
  }

  function run() {
    clear();
    beats.forEach(([t, fn]) => timers.push(setTimeout(fn, t)));
    timers.push(setTimeout(run, LOOP_MS));
  }

  const toggle = document.querySelector('[data-scene-toggle]');
  const label = document.querySelector('[data-scene-label]');

  function update() {
    const shouldPlay = !reduce.matches && !userPaused && visible;
    if (shouldPlay && !playing) {
      playing = true;
      run();
    } else if (!shouldPlay && playing) {
      playing = false;
      poster();
    }
    if (toggle) {
      toggle.hidden = reduce.matches;
      toggle.setAttribute('aria-pressed', String(userPaused));
      label.textContent = userPaused ? 'Play the demo' : 'Pause the demo';
    }
  }

  if (toggle) {
    toggle.addEventListener('click', () => {
      userPaused = !userPaused;
      update();
    });
  }

  new IntersectionObserver(
    (entries) => {
      visible = entries[0].isIntersecting;
      update();
    },
    { threshold: 0.15 },
  ).observe(frame);

  reduce.addEventListener('change', update);
  poster();
  update();
})();
