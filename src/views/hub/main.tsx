/** V1 hub view entry. */
import '../tokens.css';
import './hub.css';
// Tailwind v4 + shadcn/ui + Motion (ui-1, ADR 0104): hub-only (hub.css itself is re-imported
// inside tailwind.css, under the `hub-legacy` layer; see that file for why).
import './tailwind.css';
import { mount } from '../mount';
import { MotionConfig } from 'motion/react';
import { Hub } from './Hub';
import { Toaster } from '../ui/sonner';

mount(
  <MotionConfig reducedMotion="user">
    <Hub />
    <Toaster />
  </MotionConfig>,
);
