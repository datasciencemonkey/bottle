/**
 * The error every brain stub throws (#226). It names the component, the method and the
 * feature issue that implements it, so a call into an unbuilt part fails loudly.
 */
export class NotImplementedError extends Error {
  override readonly name = 'NotImplementedError';
  readonly component: string;
  readonly method: string;
  readonly issue: number;

  constructor(component: string, method: string, issue: number) {
    super(`${component}.${method} is not implemented yet (TODO #${issue})`);
    this.component = component;
    this.method = method;
    this.issue = issue;
  }
}
