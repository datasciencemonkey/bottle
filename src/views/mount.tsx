import { StrictMode, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';

/** Mount a thin view. Views only render and send `view_emit` (ARC-2). */
export function mount(view: ReactNode): void {
  const el = document.getElementById('root');
  if (!el) throw new Error('missing #root');
  createRoot(el).render(<StrictMode>{view}</StrictMode>);
}
