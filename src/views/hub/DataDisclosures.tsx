/**
 * Table SEC-DISC disclosures (SEC-17, AT-24): the hub "Where your data goes" panel and the same
 * copy for onboarding. Both render SEC_DISC from ./disclosures as plain text.
 * The per-MOG panel built from the live config is #94 (UX-24a); it can embed DisclosureList.
 */
import { SEC_DISC } from './disclosures';

export function DisclosureList() {
  return (
    <>
      {SEC_DISC.map((d) => (
        <article key={d.id} data-disclosure={d.id} {...(d.v1 ? { 'data-v1': '' } : {})}>
          <h3>{d.title}</h3>
          {d.lines.map((line) => (
            <p key={line}>{line}</p>
          ))}
        </article>
      ))}
    </>
  );
}

/** The hub data panel. */
export function DataPanel() {
  return (
    <section data-view="data-panel" aria-labelledby="data-panel-title">
      <h2 id="data-panel-title">Where your data goes</h2>
      <DisclosureList />
    </section>
  );
}

/**
 * The disclosures for onboarding.
 * TODO(#43): mount this in the onboarding wizard (the wizard does not exist yet).
 */
export function OnboardingDisclosures() {
  return (
    <section data-view="onboarding-disclosures" aria-labelledby="onboarding-disclosures-title">
      <h2 id="onboarding-disclosures-title">Setup: where your data goes</h2>
      <DisclosureList />
    </section>
  );
}
