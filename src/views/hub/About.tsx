/**
 * About screen (#228): product name, the Labs label (MB-10a, MB-10b) and the bundled
 * THIRD_PARTY_NOTICES.md (SEC-21b, AT-37). The notices file is the one source: Vite bundles
 * its text at build time, and the view renders it as plain text.
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
        Databricks Labs / experimental
      </p>
      {/* SEC-21b: the full notices, one click away (ui-1: a Collapsible row, same as
          DataDisclosures). forceMount keeps the <pre> in the DOM while collapsed (AT-37). */}
      <Collapsible>
        <CollapsibleTrigger className="group flex h-auto min-h-0 items-center gap-[var(--mog-space-2)] border-0 bg-transparent p-0">
          <ChevronDownIcon className="size-3.5 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
          <h3>Third-party notices</h3>
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
