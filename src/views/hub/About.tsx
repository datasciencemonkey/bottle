/**
 * About screen (#228): product name, the Labs label (MB-10a, MB-10b) and the bundled
 * THIRD_PARTY_NOTICES.md (SEC-21b, AT-37). The notices file is the one source: Vite bundles
 * its text at build time, and the view renders it as plain text.
 */
import notices from '../../../THIRD_PARTY_NOTICES.md?raw';

/** The notices text without the maintainer-only "How to add an entry" section. */
export function aboutNotices(md: string): string {
  return md.replace(/^## How to add an entry\n[\s\S]*?(?=^\| Component \||^## )/m, '');
}

export function About() {
  return (
    <section data-view="about" aria-labelledby="about-title">
      <h2 id="about-title">About Bottle</h2>
      <p data-label="labs">Databricks Labs / experimental</p>
      <h3>Third-party notices</h3>
      <pre data-notices tabIndex={0}>
        {aboutNotices(notices)}
      </pre>
    </section>
  );
}
