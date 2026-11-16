/// <reference types="vite/client" />
/** Vite build flags (`import.meta.env.DEV`), used for dev-only probes. */

interface ImportMetaEnv {
  /** `1` turns on the view latency trace and the MOG frame probe (#13, ADR 0031). */
  readonly VITE_BOTTLE_LATENCY_TRACE?: string;
}
