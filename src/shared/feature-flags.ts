/**
 * Small, view-local feature flags that are not part of the IPC contract. Each flag hides a
 * control whose host support is not ready yet, so no dead button ever renders.
 */

/** #574: the host refuses to persist a pin until the pin API ships. Hidden until then. */
export const PIN_ENABLED = false;
