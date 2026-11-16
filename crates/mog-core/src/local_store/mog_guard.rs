//! The MEM-01 guard: every local store statement filters by `mog_id`.
//!
//! The fixed statement catalog (#37) runs [`require_mog_id_filter`] on each of its statements,
//! and `exec` runs [`require_mog_id_param`] on each call. The SQL check is strict on purpose:
//! - The statement is a `SELECT`, `WITH`, `INSERT`, `REPLACE`, `UPDATE` or `DELETE`.
//! - Each `WHERE` clause has a top-level `mog_id = <bound parameter>` term (a literal does not
//!   count), and there is a `WHERE` for each `SELECT`, `UPDATE` and `DELETE`.
//! - `OR` is not allowed (it could switch the filter off), except `INSERT OR ...`.
//! - An `INSERT` names `mog_id` in its column list. The `WHERE` of its `ON CONFLICT ... DO
//!   UPDATE` acts on the conflicting row only, so it needs no second filter.
//!
//! Comments and string literals are removed before the check, so `'mog_id = ?'` does not pass.

use crate::error::{CoreError, CoreResult};

const ID: &str = "R10";

fn refused() -> CoreError {
    CoreError::MogIdRequired { component: ID }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    /// An identifier or keyword, lower case. `a.mog_id` is one token.
    Word(String),
    /// A bound parameter: `?`, `?1`, `:name`, `@name` or `$name`.
    Param,
    /// A string, blob or number literal.
    Lit,
    /// One punctuation character.
    Punct(char),
}

fn tokens(sql: &str) -> Vec<Tok> {
    let c: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let ident = |ch: char| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.';
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '-' && c.get(i + 1) == Some(&'-') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else if ch == '\'' {
            i += 1;
            loop {
                if i >= c.len() {
                    break;
                }
                if c[i] == '\'' {
                    if c.get(i + 1) == Some(&'\'') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(Tok::Lit);
        } else if ch == '"' || ch == '`' || ch == '[' {
            let close = if ch == '[' { ']' } else { ch };
            let start = i + 1;
            i += 1;
            while i < c.len() && c[i] != close {
                i += 1;
            }
            let w: String = c[start..i.min(c.len())].iter().collect();
            i += 1;
            out.push(Tok::Word(w.to_ascii_lowercase()));
        } else if ch == '?' {
            i += 1;
            while i < c.len() && c[i].is_ascii_digit() {
                i += 1;
            }
            out.push(Tok::Param);
        } else if (ch == ':' || ch == '@' || ch == '$')
            && c.get(i + 1)
                .is_some_and(|n| n.is_ascii_alphanumeric() || *n == '_')
        {
            i += 1;
            while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push(Tok::Param);
        } else if ch.is_ascii_digit() {
            while i < c.len() && ident(c[i]) {
                i += 1;
            }
            out.push(Tok::Lit);
        } else if ident(ch) {
            let start = i;
            while i < c.len() && ident(c[i]) {
                i += 1;
            }
            let w: String = c[start..i].iter().collect();
            if w.eq_ignore_ascii_case("x") && c.get(i) == Some(&'\'') {
                // A blob literal x'..': the quote part is read next as a literal.
                continue;
            }
            out.push(Tok::Word(w.to_ascii_lowercase()));
        } else {
            out.push(Tok::Punct(ch));
            i += 1;
        }
    }
    out
}

fn is_word(t: &Tok, w: &str) -> bool {
    matches!(t, Tok::Word(x) if x == w)
}

fn is_mog_col(t: &Tok) -> bool {
    matches!(t, Tok::Word(x) if x == "mog_id" || x.ends_with(".mog_id"))
}

/// Does the clause that starts after `toks[start]` (a `WHERE`) have a top-level
/// `mog_id = <param>` term?
fn where_has_mog_filter(toks: &[Tok], start: usize) -> bool {
    let mut depth = 0i32;
    let mut i = start + 1;
    while i < toks.len() {
        match &toks[i] {
            Tok::Punct('(') => depth += 1,
            Tok::Punct(')') => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            Tok::Punct(';') if depth == 0 => return false,
            Tok::Word(w)
                if depth == 0
                    && matches!(
                        w.as_str(),
                        "group"
                            | "order"
                            | "limit"
                            | "union"
                            | "except"
                            | "intersect"
                            | "returning"
                            | "window"
                    ) =>
            {
                return false
            }
            _ if depth == 0 => {
                // A whole term: it follows `WHERE` or `AND`, and no operator follows it, so
                // `1 + mog_id = ?` or `mog_id = ? + 1` does not pass.
                let starts_term = is_word(&toks[i - 1], "where") || is_word(&toks[i - 1], "and");
                let eq = matches!(toks.get(i + 1), Some(Tok::Punct('=')));
                let pair = (is_mog_col(&toks[i]) && toks.get(i + 2) == Some(&Tok::Param))
                    || (toks[i] == Tok::Param && toks.get(i + 2).is_some_and(is_mog_col));
                let ends_term = match toks.get(i + 3) {
                    None | Some(Tok::Word(_)) => true,
                    Some(Tok::Punct(p)) => matches!(p, ')' | ';'),
                    _ => false,
                };
                if starts_term && eq && pair && ends_term {
                    return true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Reject a statement that does not filter by `mog_id` (MEM-01).
pub fn require_mog_id_filter(sql: &str) -> CoreResult<()> {
    let toks = tokens(sql);
    let first = match toks.first() {
        Some(Tok::Word(w)) => w.as_str(),
        _ => return Err(refused()),
    };
    let is_insert = matches!(first, "insert" | "replace");
    if !matches!(first, "select" | "with" | "update" | "delete") && !is_insert {
        return Err(refused());
    }
    // OR can switch the filter off; only `INSERT OR <conflict>` / `UPDATE OR <conflict>` pass.
    for (i, t) in toks.iter().enumerate() {
        if is_word(t, "or") && !(i == 1 && matches!(first, "insert" | "update")) {
            return Err(refused());
        }
    }
    // The `WHERE` of `ON CONFLICT ... DO UPDATE` is exempt.
    let conflict_at = toks
        .windows(2)
        .position(|w| is_word(&w[0], "on") && is_word(&w[1], "conflict"))
        .unwrap_or(toks.len());
    if is_insert {
        let open = toks
            .iter()
            .position(|t| *t == Tok::Punct('('))
            .ok_or_else(refused)?;
        let close = toks[open..]
            .iter()
            .position(|t| *t == Tok::Punct(')'))
            .map(|p| p + open)
            .ok_or_else(refused)?;
        if !toks[open..close].iter().any(|t| is_word(t, "mog_id")) {
            return Err(refused());
        }
    }
    let mut wheres = 0usize;
    for (i, t) in toks.iter().enumerate() {
        if i < conflict_at && is_word(t, "where") {
            if !where_has_mog_filter(&toks, i) {
                return Err(refused());
            }
            wheres += 1;
        }
    }
    let needs = toks[..conflict_at]
        .iter()
        .filter(|t| is_word(t, "select"))
        .count()
        + usize::from(matches!(first, "update" | "delete"));
    if wheres < needs {
        return Err(refused());
    }
    Ok(())
}

pub(crate) fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Return the `mogId` of `exec` params, or reject the call if it is missing or not a UUID
/// (MEM-01).
pub fn require_mog_id_param(params: &serde_json::Value) -> CoreResult<&str> {
    match params.get("mogId").and_then(|v| v.as_str()) {
        Some(m) if is_uuid(m) => Ok(m),
        _ => Err(refused()),
    }
}
