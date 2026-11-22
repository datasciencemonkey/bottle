/**
 * The Home area: the calm launch screen the hub opens on. The lamp mascot (the Genie peeking out
 * of its lamp, the idle fallback art), the "Bottle" wordmark under it, and the punchline at the
 * bottom. No buttons: navigation is the sidebar, and the wordmark does nothing when clicked.
 *
 * The setup steps (Hub.tsx `SetupRegion`) render above every area, Home included, so a pending
 * step shows above the landing and a first-run user is never blocked. The landing fades in
 * gently; with reduced motion it is simply there (`data-motion="reduced"`).
 */
import { motion, useReducedMotion } from 'motion/react';
import idlePng from '../../../mog-rive/fallback/mood-idle.png?url';

export function Home({ hidden }: { hidden: boolean }) {
  const reduce = useReducedMotion();
  return (
    <section data-hub-view="home" aria-label="Home" hidden={hidden} className="hub-home">
      <motion.div
        className="hub-home-stage"
        data-motion={reduce ? 'reduced' : 'fade'}
        initial={reduce ? false : { opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={reduce ? { duration: 0 } : { duration: 0.9, ease: [0.16, 1, 0.3, 1] }}
      >
        <img
          className="hub-home-lamp"
          src={idlePng}
          alt="The Bottle mascot, a small genie, peeking out of its lamp"
          draggable={false}
        />
        <p className="hub-home-wordmark" data-home-wordmark>
          Bottle
        </p>
      </motion.div>
      <p className="hub-home-punchline" data-home-punchline>
        Ship in a bottle.
      </p>
    </section>
  );
}
