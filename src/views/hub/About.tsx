/**
 * About screen (#228): product name, the experimental label (MB-10a, MB-10b), the independent
 * open-source project notice (owner decision 2026-11-24) and the bundled THIRD_PARTY_NOTICES.md
 * (SEC-21b, AT-37). The notices file is the one source: Vite bundles its text at build time, and
 * the view renders it as plain text.
 */
import { ChevronDownIcon } from 'lucide-react';
import notices from '../../../THIRD_PARTY_NOTICES.md?raw';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../ui/collapsible';

/** The notices text without the maintainer-only "How to add an entry" section. */
export function aboutNotices(md: string): string {
  return md.replace(/^## How to add an entry\n[\s\S]*?(?=^\| Component \||^## )/m, '');
}

export function About() {
  return (
    <section data-view="about" aria-labelledby="about-title" className="hub-section">
      <h2 id="about-title">About Bottle</h2>
      <p data-label="labs" className="hub-muted">
        experimental
      </p>
      <p data-label="independent" className="hub-muted">
        Bottle is an independent open-source project and is not an official Databricks product.
      </p>
      <p data-label="trademarks" className="hub-muted">
        Databricks, Databricks Genie and Unity Catalog are trademarks of Databricks, Inc.
      </p>
      {/* SEC-21b: the full notices, one click away (ui-1: a Collapsible row, same as
          DataDisclosures). forceMount keeps the <pre> in the DOM while collapsed (AT-37). One
          consistent "show more" style across Settings (review of #602): the chevron and
          text-sm/muted-foreground label match every other disclosure trigger in Settings, not a
          heading-weight one — "Third-party notices" is the trigger's own action label, not a
          heading someone navigating the page outline needs to find, so it is a <span>, not an
          <h3>. */}
      <Collapsible>
        <CollapsibleTrigger className="group flex h-auto min-h-0 items-center gap-[var(--mog-space-2)] border-0 bg-transparent p-0 text-[var(--mog-text-sm)] text-muted-foreground hover:text-foreground">
          <ChevronDownIcon className="size-3.5 transition-transform motion-reduce:transition-none group-data-[state=open]:rotate-180" />
          <span>Third-party notices</span>
        </CollapsibleTrigger>
        <CollapsibleContent
          forceMount
          className="mt-[var(--mog-space-2)] data-[state=closed]:hidden"
        >
          <pre data-notices tabIndex={0}>
            {aboutNotices(notices)}
          </pre>
        </CollapsibleContent>
      </Collapsible>
    </section>
  );
}
