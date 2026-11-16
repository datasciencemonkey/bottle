# genie-sanitize (`src/brain/genie-sanitize`)

Domain logic for the Bottle brain window that runs before any question goes to Genie:
the question sanitizer, the secret block and the message caps. No UI, no network, no
dependencies at run time. Issue #90. T8 GenieBridge `submit` (#33) calls `prepareAsk` before
any `genie_ask`.

## Requirements covered

| ID | Rule (research/final-requirements.md) | Where |
|---|---|---|
| G-47 | Remove control, zero-width and bidi characters, code fences, HTML and markdown links. | `sanitizeQuestion` |
| G-36 | A secret pattern blocks the send with `SECRET_IN_QUESTION`. | `findSecret`, `prepareAsk` |
| G-30a | The question is 1,000 chars or less after the sanitizer; cut at a word edge with a notice. | `capQuestion` |
| G-30b | The whole message is 2,000 chars or less; preamble items drop first. | `composeMessage` |
| G-30c | The whole message is 25,000 UTF-8 bytes or less; the byte guard blocks a larger one. | `checkByteLimit`, `composeMessage` |

Every test title carries the IDs it verifies, for example `[G-30a] cuts a longer question at a word edge with a notice`.

## API

```ts
import { prepareAsk } from '../genie-sanitize';

const r = prepareAsk({ question, preamble });   // preamble: ContextPreamble of section 4.7, optional
if (r.ok) send(r.text);                          // r.notices: texts to show on the card; r.droppedItemIds
else show(r.userText);                           // r.code: 'SECRET_IN_QUESTION' | 'CLIENT_BUG'
```

`prepareAsk` runs, in this order:

1. Secret check on the raw text, on the text without invisible characters and on the sanitized text.
   A match returns `{ ok: false, code: 'SECRET_IN_QUESTION', reason: 'secret' }`. The result never holds the matched text.
2. `sanitizeQuestion(raw)`: removes C0/C1 controls (keeps `\n` and `\t`, normalizes `\r\n`, U+2028, U+2029 to `\n`),
   every Unicode format character (`\p{Cf}`: zero-width, bidi embeddings, overrides and isolates, soft hyphen, tag characters),
   script and style elements with their content, HTML comments and tags (and autolinks `<https://…>`),
   markdown inline links, images, reference links and reference definitions (the visible text stays, the URL goes;
   `src/links.ts` is a linear scanner, so balanced brackets in the text, balanced parentheses in the URL, titles with
   parentheses, `<…>` destinations, nested labels, definitions with the URL on the next line and definitions after
   block quote or list markers (`> [r]: …`, `- [r]: …`) are all covered;
   a stray `](…)` without its opening bracket is removed too),
   and code fence delimiters (backtick and tilde runs of 3 or more, with the info string of a fence line).
   The passes repeat until the text does not change, so nested markup cannot rebuild itself. A whole run of `<`
   before a tag start is then removed. If 32 passes do not reach a fixpoint (markup nested deeper than that), the
   sanitizer fails closed and removes every `<`, `>`, `[`, `]`, backtick and tilde. Then spaces collapse.
   An empty result returns `{ ok: false, code: 'CLIENT_BUG', reason: 'empty' }`.
3. `capQuestion(text)`: over 1,000 code points, cuts at the last whitespace (a hard cut at 1,000 code points if there is none)
   and gives the notice "Bottle shortened the question to 1,000 characters."
4. `composeMessage(question, preamble?)`: renders `header`, one `\n- <text>` line per item, a blank line, then the question.
   With no preamble (or no items) the text is the question exactly. Over 2,000 code points it drops items, the last item first,
   then the header; it reports `droppedItemIds` and a notice. Then `checkByteLimit` blocks over 25,000 UTF-8 bytes
   (`CLIENT_BUG`, reason `message_bytes`). The returned `preamble.chars` is the length of the rendered preamble block.

Other exports: `findSecret(text)` (returns the pattern name), `SECRET_PATTERNS`, `stripInvisible`, `charCount` (code points),
`utf8ByteLength`, `renderMessage`, `checkByteLimit`, and the constants `QUESTION_MAX_CHARS`, `MESSAGE_MAX_CHARS`, `MESSAGE_MAX_BYTES`.

Secret patterns (section 4.6 rule 7, unchanged): `eyJ[\w-]+\.`, `dapi[0-9a-f]{32}`, `dose[0-9a-f]+`, `AKIA[0-9A-Z]{16}`,
`ghp_[A-Za-z0-9]+`, `-----BEGIN`, `password=`.

## Test

The root checks cover it (App CI runs them on macOS and Windows):

```sh
pnpm typecheck
pnpm lint
pnpm exec vitest run src/brain/genie-sanitize
```

Test fixtures use only obviously fake values (for example `dapi` + 32 zeros), built at run time.

## Out of scope

- Preamble building, item caps and truncation (G-29a..G-29d), item filters (G-31, Table G-CTX), PII scrub and
  dropping of secret items (G-37a..G-37c), turn rules (G-32a, G-32b): the ContextAssembler issue.
- The ContextGate approval binding (G-34a, G-34b) and the POLICY_DENIED retry (G-33a, G-33b): the ContextGate and GenieBridge issues.
- Wiring into `GenieBridge.submit` and the card UI that shows the notices: the Genie bridge and card issues.
- The Rust outbound scrub of section 4.6 rule 7: the Rust core (`crates/mog-core`).

## Known limits

- The `dose[0-9a-f]+` pattern is kept exactly as section 4.6 states it, so ordinary words such as "dosed" also block the send.
  A tighter pattern needs a requirements change.
- Decision: anything that a browser would parse as an HTML tag (`<` + letter, up to the next `>`) is removed, even when it
  was meant as prose: `a<b and c>d` becomes `ad`. This errs on the side of removal; `<` followed by a digit or a space stays.
- Decision: bare URLs (`https://…`, `www.…`) are plain text, not markdown link syntax, so G-47 does not remove them.
  Only the `[text](url)`, reference, image and `<url>` autolink forms count as markdown links here.
- Markdown nested deeper than 64 levels of link text is dropped with its text.
- "chars" are Unicode code points. A hard cut can split a grapheme cluster (for example a flag or a combining mark).
