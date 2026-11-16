/** V2 MOG view (thin, one mascot instance). TODO(#226): view state and view_emit. */
import { useEffect, useRef } from 'react';
import { mount } from '../mount';
import { PlaceholderMascot } from './mascot';

function MogView() {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const mascot = new PlaceholderMascot();
    mascot.mount(el);
    return () => mascot.destroy();
  }, []);
  return <div ref={ref} aria-label="MOG" />;
}

mount(<MogView />);
