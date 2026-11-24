/** V1 hub view entry. */
import '../tokens.css';
import './hub.css';
// Tailwind v4 + shadcn/ui + Motion (ui-1, ADR 0106): hub-only. hub.css (imported just above)
// declares the five-layer cascade order (`@layer theme, base, hub-legacy, components,
// utilities;`) and puts its own rules in `hub-legacy`, between `base` and `components`, before
// Tailwind ever loads; tailwind.css's own `@import 'tailwindcss'` then registers `theme`,
// `base`, `components` and `utilities` into that order rather than re-declaring it. See
// hub.css's own top comment for why that ordering keeps it winning over Tailwind's preflight
// while a shadcn component's utilities still win over a matching hub.css selector.
import './tailwind.css';
// Slice 2a: the agent turn body is shared with the card window (plain CSS on tokens.css).
import '../card/agent-run.css';
import { mount } from '../mount';
import { MotionConfig } from 'motion/react';
import { Hub } from './Hub';

mount(
  <MotionConfig reducedMotion="user">
    <Hub />
  </MotionConfig>,
);
