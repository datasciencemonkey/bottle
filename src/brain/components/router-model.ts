/**
 * The router model per region (#135; RTR-10a, RTR-10b, D38, D58, ADR 0065).
 *
 * Llama 3.1 8B runs the router step in a region that serves it, and only when its strict-schema
 * eval (SP-13) passed. Every other case uses `gpt-oss-20b`. The model name is an eval-gated
 * value (NFR-22a): a model change needs an eval record.
 */

export const LLAMA_3_1_8B = 'system.ai.meta-llama-3-1-8b-instruct';
export const GPT_OSS_20B = 'system.ai.gpt-oss-20b';

export type RouterModel = typeof LLAMA_3_1_8B | typeof GPT_OSS_20B;

/** The strict-schema eval result per model. A missing entry means the eval has not passed. */
export type RouterEvalRecord = Readonly<Partial<Record<RouterModel, boolean>>>;

/**
 * The eval record until SP-13 runs: no model has passed.
 * TODO(#56): SP-13 writes the real record (≥ 99.5 % valid JSON, intent accuracy ≥ 90 %).
 */
export const PENDING_ROUTER_EVAL: RouterEvalRecord = Object.freeze({});

/** Azure US regions (`eastus`, `eastus2`, `westus3`, `southcentralus`, ...). */
const AZURE_US = /^(?:east|west|central|northcentral|southcentral|westcentral)us\d?$/;

/**
 * Whether a cloud region serves Llama 3.1 8B. US regions (AWS and GCP `us-*`, Azure `*us`) do;
 * EU regions (`eu-west-1`, `eu-central-1`, COL-51) and all others do not.
 * TODO(#56): SP-13 confirms the served-region list, including GovCloud (`us-gov-*`), which this
 * rule treats as US.
 */
export function servesLlama318b(region: string): boolean {
  const r = region.trim().toLowerCase();
  return r.startsWith('us-') || AZURE_US.test(r);
}

/** The router model for `region` (RTR-10a, RTR-10b). */
export function routerModelFor(region: string, evals: RouterEvalRecord): RouterModel {
  return servesLlama318b(region) && evals[LLAMA_3_1_8B] === true ? LLAMA_3_1_8B : GPT_OSS_20B;
}
