//! The virtual memory folder `/memories` (local memory plan 5.2, 5.3; ADR 0112).
//!
//! No real folder exists. Each memory is one `memory_item` row, and Rust computes its file form
//! (YAML frontmatter, then a Markdown body) and the `index.md` of each folder from the columns.
//! The hub, the export and the agent memory tool all see this form. This module holds:
//! - the memory kinds and their kind folders;
//! - the slug rules of a memory and of a MOG folder;
//! - the fixed path grammar `/memories/(shared|<mog-slug>)/[<kind-folder>/<slug>.md | index.md]`
//!   (anything else is refused, so a path can never point outside the store);
//! - the renderers of one memory file and of one folder index.
//!
//! Logs: nothing here logs. A rendered file is user content (SEC-16c).

use chrono::{DateTime, SecondsFormat};
use serde_json::Value;

/// The root of the virtual folder.
pub const ROOT: &str = "/memories";
/// The reserved folder name of the shared folder. No MOG folder can take it.
pub const SHARED: &str = "shared";
/// The most characters of a slug.
pub const SLUG_MAX: usize = 64;
/// The most lines of one index file (plan 5.3).
pub const INDEX_MAX_LINES: usize = 200;
/// The most bytes of one index file (plan 5.3).
pub const INDEX_MAX_BYTES: usize = 25 * 1024;
/// The most characters of the summary on one index line (plan 5.3).
pub const INDEX_SUMMARY_MAX: usize = 80;

/// The kind of a memory (plan 5.1). `entity` and `person` are not kinds (owner, Q7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Fact,
    Preference,
    Definition,
    Decision,
    OpenQuestion,
    GenieAnswer,
}

impl Kind {
    /// Every kind, in the order of the index and the hub tree.
    pub const ALL: [Kind; 6] = [
        Kind::Fact,
        Kind::Definition,
        Kind::Decision,
        Kind::OpenQuestion,
        Kind::GenieAnswer,
        Kind::Preference,
    ];

    /// The `item_type` code of the store.
    pub fn code(self) -> &'static str {
        match self {
            Kind::Fact => "fact",
            Kind::Preference => "preference",
            Kind::Definition => "definition",
            Kind::Decision => "decision",
            Kind::OpenQuestion => "open_question",
            Kind::GenieAnswer => "genie_answer",
        }
    }

    /// The kind folder of the path.
    pub fn folder(self) -> &'static str {
        match self {
            Kind::Fact => "facts",
            Kind::Preference => "preferences",
            Kind::Definition => "definitions",
            Kind::Decision => "decisions",
            Kind::OpenQuestion => "open-questions",
            Kind::GenieAnswer => "answers",
        }
    }

    pub fn from_code(code: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.code() == code)
    }

    pub fn from_folder(folder: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.folder() == folder)
    }
}

/// The slug of `text`: lower-case ASCII letters and digits, with one hyphen for each run of
/// other characters, at most [`SLUG_MAX`] characters, cut at a hyphen when it can be. Accents
/// fold to their base letter (NFKD, then the combining marks go). An empty result is `memory`.
pub fn slugify(text: &str) -> String {
    let nfkd = icu_normalizer::DecomposingNormalizerBorrowed::new_nfkd().normalize(text);
    let mut out = String::new();
    let mut gap = false;
    for c in nfkd.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.push(c.to_ascii_lowercase());
        } else if !is_combining_mark(c) {
            gap = true;
        }
    }
    if out.len() > SLUG_MAX {
        out.truncate(SLUG_MAX);
        if let Some(cut) = out.rfind('-').filter(|&i| i >= SLUG_MAX / 2) {
            out.truncate(cut);
        }
        while out.ends_with('-') {
            out.pop();
        }
    }
    if out.is_empty() {
        "memory".to_owned()
    } else {
        out
    }
}

/// The combining marks that NFKD splits off a Latin letter (U+0300..U+036F and the extended
/// blocks). A slug drops them, so `Café` becomes `cafe`.
fn is_combining_mark(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036f}' | '\u{1ab0}'..='\u{1aff}' | '\u{1dc0}'..='\u{1dff}' | '\u{20d0}'..='\u{20ff}' | '\u{fe20}'..='\u{fe2f}')
}

/// True if `s` is a well-formed slug: 1 to 72 characters (a 64-character base plus a suffix) of
/// `a-z`, `0-9` and single inner hyphens.
pub fn is_slug(s: &str) -> bool {
    (1..=SLUG_MAX + 8).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
}

/// The first `n` hex digits of a UUID, for a clash suffix.
fn id_prefix(id: &str, n: usize) -> String {
    id.chars()
        .filter(char::is_ascii_hexdigit)
        .take(n)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// The file slug of a memory: its stored slug, or `item-<first 8 of id>` for a row that has none
/// (a row from before migration 0003).
pub fn item_slug(stored: Option<&str>, item_id: &str) -> String {
    match stored {
        Some(s) if is_slug(s) => s.to_owned(),
        _ => format!("item-{}", id_prefix(item_id, 8)),
    }
}

/// The folder slug of each MOG on the device, in the order of `mogs` (`(mog_id, display_name)`).
/// The base is the slug of the name. A clash with another MOG, or with the reserved name
/// [`SHARED`], adds `-<first 6 of mog_id>`. So each folder name is unique on the device.
pub fn mog_slugs(mogs: &[(String, String)]) -> Vec<(String, String)> {
    let bases: Vec<String> = mogs.iter().map(|(_, name)| slugify(name)).collect();
    mogs.iter()
        .zip(&bases)
        .map(|((id, _), base)| {
            let clash = base == SHARED || bases.iter().filter(|b| *b == base).count() > 1;
            let slug = if clash {
                format!("{base}-{}", id_prefix(id, 6))
            } else {
                base.clone()
            };
            (id.clone(), slug)
        })
        .collect()
}

/// One parsed path of the virtual folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VPath {
    /// `/memories` or `/memories/index.md`: the list of folders.
    Root,
    /// `/memories/<folder>/index.md` (or the folder itself).
    Index { folder: String },
    /// `/memories/<folder>/<kind-folder>/<slug>.md`.
    File {
        folder: String,
        kind: Kind,
        slug: String,
    },
}

/// Parse a path by the fixed grammar. Anything else is `None`: `..`, `.`, a double slash, a
/// backslash, a percent form, an unknown kind folder or a bad slug.
pub fn parse_path(path: &str) -> Option<VPath> {
    let rest = path.strip_prefix(ROOT)?;
    if rest.is_empty() || rest == "/" || rest == "/index.md" {
        return Some(VPath::Root);
    }
    let rest = rest.strip_prefix('/')?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let parts: Vec<&str> = rest.split('/').collect();
    let folder_ok = |f: &str| is_slug(f);
    match parts.as_slice() {
        [folder] | [folder, "index.md"] if folder_ok(folder) => Some(VPath::Index {
            folder: (*folder).to_owned(),
        }),
        [folder, kind, file] if folder_ok(folder) => {
            let kind = Kind::from_folder(kind)?;
            let slug = file.strip_suffix(".md")?;
            if !is_slug(slug) || slug == "index" {
                return None;
            }
            Some(VPath::File {
                folder: (*folder).to_owned(),
                kind,
                slug: slug.to_owned(),
            })
        }
        _ => None,
    }
}

/// The path of one memory file.
pub fn file_path(folder: &str, kind: Kind, slug: &str) -> String {
    format!("{ROOT}/{folder}/{}/{slug}.md", kind.folder())
}

/// The path of the index of one folder.
pub fn index_path(folder: &str) -> String {
    format!("{ROOT}/{folder}/index.md")
}

/// One memory as the renderer reads it: the columns of `mem.item.list` / `mem.item.get`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MemoryRow {
    pub item_id: String,
    pub kind: String,
    pub title: String,
    /// The summary line (`text`).
    pub text: String,
    pub body_md: String,
    pub slug: Option<String>,
    pub status: String,
    pub pinned: bool,
    pub source: String,
    pub origin: String,
    pub tags: Vec<String>,
    pub confidence: f64,
    pub sensitivity: String,
    pub workspace_host: Option<String>,
    pub workspace_id: Option<String>,
    pub profile: Option<String>,
    pub exchange_id: Option<String>,
    pub recording_id: Option<String>,
    pub superseded_by: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub last_used_ms: Option<i64>,
}

impl MemoryRow {
    /// Read one row of `{rows: [...]}` (camelCase keys). `None` if a required key is missing.
    pub fn from_json(v: &Value) -> Option<MemoryRow> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        let opt = |k: &str| -> Option<String> {
            match v.get(k)? {
                Value::String(x) => Some(x.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            }
        };
        let tags = match v.get("tags") {
            Some(Value::String(t)) => serde_json::from_str::<Vec<String>>(t).unwrap_or_default(),
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        };
        let text = s("text")?;
        Some(MemoryRow {
            item_id: s("itemId")?,
            kind: s("kind")?,
            title: s("title").unwrap_or_else(|| text.chars().take(80).collect()),
            body_md: s("bodyMd").unwrap_or_default(),
            text,
            slug: s("slug"),
            status: s("status")?,
            pinned: matches!(v.get("pinned"), Some(Value::Bool(true)))
                || v.get("pinned").and_then(Value::as_i64) == Some(1),
            source: s("source").unwrap_or_default(),
            origin: s("origin").unwrap_or_else(|| "legacy".to_owned()),
            tags,
            confidence: v.get("confidence").and_then(Value::as_f64).unwrap_or(0.0),
            sensitivity: s("sensitivity").unwrap_or_else(|| "normal".to_owned()),
            workspace_host: s("workspaceHost"),
            workspace_id: opt("workspaceId"),
            profile: s("profile"),
            exchange_id: s("exchangeId"),
            recording_id: s("recordingId"),
            superseded_by: s("supersededBy"),
            created_ms: v.get("createdAt").and_then(Value::as_i64).unwrap_or(0),
            updated_ms: v.get("updatedAt").and_then(Value::as_i64).unwrap_or(0),
            last_used_ms: v.get("lastUsedAt").and_then(Value::as_i64),
        })
    }

    pub fn kind(&self) -> Option<Kind> {
        Kind::from_code(&self.kind)
    }

    pub fn slug(&self) -> String {
        item_slug(self.slug.as_deref(), &self.item_id)
    }

    /// The path of this memory in the folder `folder`. `None` for an unknown kind.
    pub fn path(&self, folder: &str) -> Option<String> {
        Some(file_path(folder, self.kind()?, &self.slug()))
    }

    /// The time the index sorts by: the last use, else the last update.
    fn recency(&self) -> i64 {
        self.last_used_ms
            .unwrap_or(self.updated_ms)
            .max(self.updated_ms)
    }
}

/// A YAML scalar: a JSON string (valid YAML), so a `:`, `#`, quote or new line in user text can
/// never start a new key.
fn quoted(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_owned())
}

fn opt_quoted(s: Option<&str>) -> String {
    s.map_or_else(|| "null".to_owned(), quoted)
}

fn iso(ms: i64) -> String {
    DateTime::from_timestamp_millis(ms).map_or_else(
        || "null".to_owned(),
        |d| d.to_rfc3339_opts(SecondsFormat::Secs, true),
    )
}

/// The file form of one memory (plan 5.2): the frontmatter fields of the plan, in its order,
/// then the body. The body is `body_md`, or the summary when the body is empty.
pub fn render_file(row: &MemoryRow, folder: &str) -> String {
    let tags = format!(
        "[{}]",
        row.tags
            .iter()
            .map(|t| quoted(t))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let supersedes = "[]";
    let lines = [
        format!("id: {}", row.item_id),
        format!("kind: {}", row.kind),
        format!("title: {}", quoted(&row.title)),
        format!("status: {}", row.status),
        format!("pinned: {}", row.pinned),
        format!("source: {}", row.source),
        format!("origin: {}", row.origin),
        format!("mog: {folder}"),
        format!(
            "workspace_host: {}",
            opt_quoted(row.workspace_host.as_deref())
        ),
        format!("workspace_id: {}", opt_quoted(row.workspace_id.as_deref())),
        format!("profile: {}", opt_quoted(row.profile.as_deref())),
        format!(
            "exchange_id: {}",
            row.exchange_id.as_deref().unwrap_or("null")
        ),
        format!(
            "recording_id: {}",
            row.recording_id.as_deref().unwrap_or("null")
        ),
        format!("created: {}", iso(row.created_ms)),
        format!("updated: {}", iso(row.updated_ms)),
        format!(
            "last_used: {}",
            row.last_used_ms.map_or_else(|| "null".to_owned(), iso)
        ),
        format!("confidence: {}", row.confidence),
        format!("sensitivity: {}", row.sensitivity),
        format!("tags: {tags}"),
        format!("supersedes: {supersedes}"),
    ];
    let body = if row.body_md.trim().is_empty() {
        row.text.as_str()
    } else {
        row.body_md.as_str()
    };
    format!("---\n{}\n---\n{}\n", lines.join("\n"), body.trim_end())
}

/// One line of text: each run of white space is one space, at most `max` characters, with `…`
/// when it is cut.
fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    cut = cut.trim_end().to_owned();
    cut.push('…');
    cut
}

/// The link text of a Markdown link: no `[`, `]` or new line can close it early.
fn link_text(s: &str) -> String {
    one_line(s, 80).replace('[', "(").replace(']', ")")
}

/// The statuses that an index lists. `superseded`, `retracted` and `removed` memories are not
/// in use, so the index leaves them out; the hub shows them in its own lists.
fn listed(status: &str) -> bool {
    matches!(status, "active" | "proposed" | "stale" | "quarantined")
}

/// The `index.md` of one folder (plan 5.3): a title line, then one line per listed memory,
/// pinned first, then by the last use. At most [`INDEX_MAX_LINES`] lines and
/// [`INDEX_MAX_BYTES`] bytes; a last line names how many memories it left out.
pub fn render_index(folder: &str, rows: &[MemoryRow]) -> String {
    let mut items: Vec<&MemoryRow> = rows
        .iter()
        .filter(|r| listed(&r.status) && r.kind().is_some())
        .collect();
    items.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.recency().cmp(&a.recency()))
            .then(a.item_id.cmp(&b.item_id))
    });
    let head = format!("# {folder}\n\n");
    let mut out = head.clone();
    let mut lines = 1usize;
    let mut shown = 0usize;
    for r in &items {
        let Some(kind) = r.kind() else { continue };
        let mark = match r.status.as_str() {
            "proposed" => " (proposed)",
            "stale" => " (stale)",
            "quarantined" => " (quarantined)",
            _ => "",
        };
        let line = format!(
            "- [{}]({}/{}.md){mark} — {}\n",
            link_text(&r.title),
            kind.folder(),
            r.slug(),
            one_line(&r.text, INDEX_SUMMARY_MAX)
        );
        // Keep room for the "more" line.
        if lines + 2 > INDEX_MAX_LINES || out.len() + line.len() + 64 > INDEX_MAX_BYTES {
            break;
        }
        out.push_str(&line);
        lines += 1;
        shown += 1;
    }
    if shown == 0 {
        out.push_str("No memories yet.\n");
    } else if shown < items.len() {
        out.push_str(&format!("- … and {} more\n", items.len() - shown));
    }
    out
}

/// The `/memories/index.md` root: one line per folder, `(folder, count)`.
pub fn render_root(folders: &[(String, usize)]) -> String {
    let mut out = String::from("# memories\n\n");
    if folders.is_empty() {
        out.push_str("No memories yet.\n");
    }
    for (f, n) in folders {
        out.push_str(&format!("- [{f}]({f}/index.md) — {n} memories\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, kind: &str, title: &str) -> MemoryRow {
        MemoryRow {
            item_id: id.to_owned(),
            kind: kind.to_owned(),
            title: title.to_owned(),
            text: format!("{title} summary"),
            slug: Some(slugify(title)),
            status: "active".to_owned(),
            source: "you".to_owned(),
            origin: "pin".to_owned(),
            confidence: 1.0,
            sensitivity: "normal".to_owned(),
            created_ms: 1_795_000_000_000,
            updated_ms: 1_795_000_000_000,
            ..MemoryRow::default()
        }
    }

    const ID_A: &str = "7f0c2e1a-3d4b-4c5e-9f60-1a2b3c4d5e6f";
    const ID_B: &str = "0a1b2c3d-0000-4000-8000-000000000002";

    #[test]
    fn slugify_rules() {
        assert_eq!(slugify("Active user"), "active-user");
        assert_eq!(
            slugify("  The mart refreshes at six! "),
            "the-mart-refreshes-at-six"
        );
        assert_eq!(slugify("Café crème"), "cafe-creme");
        assert_eq!(slugify("Q3 churn: new formula"), "q3-churn-new-formula");
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd");
        assert_eq!(slugify("日本語"), "memory");
        assert_eq!(slugify(""), "memory");
        let long = slugify(&"word ".repeat(40));
        assert!(long.len() <= SLUG_MAX && is_slug(&long), "{long}");
    }

    #[test]
    fn every_slugify_result_is_a_slug() {
        for t in [
            "a", "A--B", "-x-", "x__y", "ÅÄÖ", "1.2.3", "%2e%2e", "a/b\\c",
        ] {
            assert!(is_slug(&slugify(t)), "{t}");
        }
    }

    #[test]
    fn item_slug_falls_back_to_the_id() {
        assert_eq!(item_slug(Some("active-user"), ID_A), "active-user");
        assert_eq!(item_slug(None, ID_A), "item-7f0c2e1a");
        assert_eq!(item_slug(Some("Bad Slug"), ID_A), "item-7f0c2e1a");
    }

    #[test]
    fn mog_slugs_are_unique_and_never_shared() {
        let mogs = vec![
            (ID_A.to_owned(), "Sales MOG".to_owned()),
            (ID_B.to_owned(), "sales mog".to_owned()),
            (
                "11111111-2222-4333-8444-555555555555".to_owned(),
                "Shared".to_owned(),
            ),
            (
                "99999999-2222-4333-8444-555555555555".to_owned(),
                "Finance".to_owned(),
            ),
        ];
        let s = mog_slugs(&mogs);
        assert_eq!(s[0].1, "sales-mog-7f0c2e");
        assert_eq!(s[1].1, "sales-mog-0a1b2c");
        assert_eq!(s[2].1, "shared-111111");
        assert_eq!(s[3].1, "finance");
    }

    #[test]
    fn ac_1_7_the_path_of_a_fact() {
        let r = row(ID_A, "fact", "The mart refreshes at six");
        assert_eq!(
            r.path("mog-a").as_deref(),
            Some("/memories/mog-a/facts/the-mart-refreshes-at-six.md")
        );
        let a = row(ID_B, "genie_answer", "Churn in Q3");
        assert_eq!(
            a.path("mog-a").as_deref(),
            Some("/memories/mog-a/answers/churn-in-q3.md")
        );
    }

    #[test]
    fn parse_path_accepts_the_grammar() {
        assert_eq!(parse_path("/memories"), Some(VPath::Root));
        assert_eq!(parse_path("/memories/"), Some(VPath::Root));
        assert_eq!(parse_path("/memories/index.md"), Some(VPath::Root));
        assert_eq!(
            parse_path("/memories/mog-a/index.md"),
            Some(VPath::Index {
                folder: "mog-a".into()
            })
        );
        assert_eq!(
            parse_path("/memories/mog-a"),
            Some(VPath::Index {
                folder: "mog-a".into()
            })
        );
        assert_eq!(
            parse_path("/memories/mog-a/facts/a-b.md"),
            Some(VPath::File {
                folder: "mog-a".into(),
                kind: Kind::Fact,
                slug: "a-b".into()
            })
        );
        assert_eq!(
            parse_path("/memories/shared/preferences/eur.md"),
            Some(VPath::File {
                folder: "shared".into(),
                kind: Kind::Preference,
                slug: "eur".into()
            })
        );
    }

    #[test]
    fn parse_path_refuses_everything_else() {
        for p in [
            "",
            "/",
            "memories",
            "/memoriesx",
            "/memories/../x",
            "/memories/mog-a/../b/facts/x.md",
            "/memories/./facts/x.md",
            "/memories//facts/x.md",
            "/memories/mog-a/facts/../x.md",
            "/memories/mog-a/facts/x",
            "/memories/mog-a/facts/x.txt",
            "/memories/mog-a/people/x.md",
            "/memories/mog-a/facts/X.md",
            "/memories/mog-a/facts/index.md",
            "/memories/mog-a/facts/a/b.md",
            "/memories/mog-a/facts/%2e%2e.md",
            "/memories/mog-a\\facts\\x.md",
            "/memories/Mog-A/index.md",
            "/etc/passwd",
        ] {
            assert_eq!(parse_path(p), None, "{p}");
        }
    }

    #[test]
    fn render_file_has_the_plan_fields_in_order() {
        let mut r = row(ID_A, "definition", "Active user");
        r.pinned = true;
        r.tags = vec!["metrics".into(), "growth".into()];
        r.workspace_host = Some("<host-a>".into());
        r.workspace_id = Some("1234567890".into());
        r.profile = Some("profile-a".into());
        r.body_md = "An active user logged in this month.".into();
        let f = render_file(&r, "mog-a");
        let keys: Vec<&str> = f
            .lines()
            .skip(1)
            .take_while(|l| *l != "---")
            .filter_map(|l| l.split(':').next())
            .collect();
        assert_eq!(
            keys,
            [
                "id",
                "kind",
                "title",
                "status",
                "pinned",
                "source",
                "origin",
                "mog",
                "workspace_host",
                "workspace_id",
                "profile",
                "exchange_id",
                "recording_id",
                "created",
                "updated",
                "last_used",
                "confidence",
                "sensitivity",
                "tags",
                "supersedes"
            ]
        );
        assert!(f.contains("title: \"Active user\"\n"));
        assert!(f.contains("workspace_id: \"1234567890\"\n"));
        assert!(f.contains("pinned: true\n"));
        assert!(f.contains("tags: [\"metrics\", \"growth\"]\n"));
        assert!(f.contains("created: 2026-11-18T"));
        assert!(f.ends_with("---\nAn active user logged in this month.\n"));
    }

    #[test]
    fn render_file_quotes_user_text_so_it_cannot_add_a_key() {
        let mut r = row(ID_A, "fact", "x\nstatus: active\n---");
        r.status = "quarantined".into();
        let f = render_file(&r, "mog-a");
        let front: Vec<&str> = f.lines().skip(1).take_while(|l| *l != "---").collect();
        assert_eq!(front.iter().filter(|l| l.starts_with("status:")).count(), 1);
        assert!(front.contains(&"status: quarantined"));
    }

    #[test]
    fn render_file_uses_the_summary_when_the_body_is_empty() {
        let r = row(ID_A, "fact", "Mart");
        assert!(render_file(&r, "mog-a").ends_with("---\nMart summary\n"));
    }

    #[test]
    fn render_index_orders_by_pin_then_last_use_and_marks_status() {
        let mut a = row(ID_A, "fact", "Older pinned");
        a.pinned = true;
        let mut b = row(ID_B, "decision", "Newer");
        b.last_used_ms = Some(1_796_000_000_000);
        let mut q = row("33333333-0000-4000-8000-000000000003", "fact", "Bad one");
        q.status = "quarantined".into();
        let mut gone = row("44444444-0000-4000-8000-000000000004", "fact", "Gone");
        gone.status = "superseded".into();
        let idx = render_index("mog-a", &[b, gone, q, a]);
        let lines: Vec<&str> = idx.lines().filter(|l| l.starts_with("- ")).collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("- [Older pinned](facts/older-pinned.md) — "));
        assert!(lines[1].starts_with("- [Newer](decisions/newer.md) — "));
        assert!(lines[2].contains("(quarantined)"));
        assert!(!idx.contains("Gone"));
    }

    #[test]
    fn render_index_keeps_the_caps() {
        let rows: Vec<MemoryRow> = (0..400)
            .map(|i| {
                let mut r = row(
                    &format!("{i:08x}-0000-4000-8000-000000000000"),
                    "fact",
                    &format!("Memory {i} {}", "long title ".repeat(6)),
                );
                r.text = "s".repeat(600);
                r.slug = Some(format!("m-{i}"));
                r
            })
            .collect();
        let idx = render_index("mog-a", &rows);
        assert!(idx.lines().count() <= INDEX_MAX_LINES + 2);
        assert!(idx.len() <= INDEX_MAX_BYTES);
        assert!(idx.contains("more\n"));
        for l in idx.lines().filter(|l| l.starts_with("- [")) {
            let summary = l.rsplit(" — ").next().unwrap_or("");
            assert!(summary.chars().count() <= INDEX_SUMMARY_MAX, "{l}");
        }
    }

    #[test]
    fn index_link_text_cannot_break_the_link() {
        let r = row(ID_A, "fact", "a](http://x) [b");
        let idx = render_index("mog-a", &[r]);
        assert!(idx.contains("- [a)(http://x) (b]("));
    }

    #[test]
    fn row_from_json_reads_the_list_columns() {
        let v = serde_json::json!({
            "itemId": ID_A, "kind": "fact", "title": "T", "text": "S", "slug": "t",
            "status": "active", "pinned": 1, "source": "you", "origin": "pin",
            "tags": "[\"a\"]", "confidence": 1.0, "workspaceId": 1234567890_i64,
            "createdAt": 1, "updatedAt": 2
        });
        let r = MemoryRow::from_json(&v).expect("row");
        assert!(r.pinned);
        assert_eq!(r.tags, ["a"]);
        assert_eq!(r.workspace_id.as_deref(), Some("1234567890"));
        assert_eq!(r.path("f").as_deref(), Some("/memories/f/facts/t.md"));
    }

    #[test]
    fn render_root_lists_folders() {
        let r = render_root(&[("mog-a".into(), 3)]);
        assert!(r.contains("- [mog-a](mog-a/index.md) — 3 memories\n"));
        assert!(render_root(&[]).contains("No memories yet."));
    }
}
