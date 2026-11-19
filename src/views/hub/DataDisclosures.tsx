/**
 * Table SEC-DISC disclosures (SEC-17, AT-24): the hub "Where your data goes" panel and the same
 * copy for onboarding. Both render SEC_DISC from ./disclosures as plain text.
 * The per-MOG panel built from the live config is #94 (UX-24a); it can embed DisclosureList.
 */
import { ChevronDownIcon } from 'lucide-react';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../ui/collapsible';
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
    <section data-view="data-panel" aria-labelledby="data-panel-title" className="hub-section">
      {/* SEC-17: every disclosure stays in the panel, behind one labelled entry point (ui-1: a
          Collapsible disclosure row with an animated chevron, instead of a native <details>). */}
      <Collapsible>
        <CollapsibleTrigger className="group flex h-auto min-h-0 items-center gap-[var(--mog-space-2)] border-0 bg-transparent p-0">
          <ChevronDownIcon className="size-3.5 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
          <h2 id="data-panel-title">Where your data goes</h2>
        </CollapsibleTrigger>
        {/* forceMount (ui-1): like the native <details> this replaces, the disclosure text stays
            in the DOM even while collapsed (`data-[state=closed]:hidden`), not unmounted — SEC-17
            and AT-24 read every item regardless of the open state. */}
        <CollapsibleContent
          forceMount
          className="mt-[var(--mog-space-3)] data-[state=closed]:hidden"
        >
          <DisclosureList />
        </CollapsibleContent>
      </Collapsible>
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
