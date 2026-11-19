/**
 * shadcn/ui's `cn()` helper (ui-1, ADR 0104): merges conditional class names (clsx) and resolves
 * conflicting Tailwind utility classes (tailwind-merge), so a caller can override a component's
 * default classes by passing a `className` prop. Hub-only: nothing outside src/views/ui and the
 * hub imports Tailwind utility classes.
 */
import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
