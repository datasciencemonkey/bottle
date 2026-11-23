//! Host-minted user-action tickets (#574, MEM-28b; ADR 0113).
//!
//! Some memory writes need proof of a user action, and the brain cannot give that proof: it is
//! code, not a user. So the host mints a ticket on the user's own action and the brain passes it
//! back with the write. The host redeems it before the write runs:
//!
//! | Trigger | Minted on | Purpose | Binds |
//! |---|---|---|---|
//! | `Ui` | a card `memory.remember` click | `Pin` | the MOG only |
//! | `Voice` | Rust's own transcript holds "remember this: …" | `Pin` | `[text]` |
//! | `HubAdd` | a hub `memory.add` click | `Pin` | `["add", kind, title, body]` |
//! | `HubRate` | a hub `memory.rate` click | `Rate` | `[itemId]` |
//! | `HubEdit` | a hub `memory.edit` click | `Edit` | `["edit", itemId, kind, title, body]` |
//!
//! The binding is the SHA-256 of the compact JSON array. At redeem time the ticket builds the
//! array again from the write params (`itemType` is the kind), so the write must hold exactly
//! what the user clicked or said. A ticket is single-use, bound to one MOG, and expires after
//! [`TICKET_TTL_MS`]. [`ActionTickets::redeem`] takes it out of the map first, so a replay or a
//! second try after a refusal is refused. The map holds at most [`MAX_OPEN`] tickets; a new one
//! drops the oldest. A ticket holds no text, only a hash.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// How long a ticket is valid.
pub const TICKET_TTL_MS: u64 = 60_000;
/// The most open tickets.
pub const MAX_OPEN: usize = 16;

/// What the user did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// A click on "Remember this" on the card.
    Ui,
    /// The voice command "remember this: <text>".
    Voice,
    /// "New memory" saved in the hub.
    HubAdd,
    /// A thumbs up in the hub (it also releases a quarantined item).
    HubRate,
    /// An edit saved in the hub.
    HubEdit,
}

/// What a ticket allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// A pinned `mem.item.write`.
    Pin,
    /// `mem.item.rate`.
    Rate,
    /// `mem.item.edit`.
    Edit,
}

impl Trigger {
    pub fn purpose(self) -> Purpose {
        match self {
            Trigger::Ui | Trigger::Voice | Trigger::HubAdd => Purpose::Pin,
            Trigger::HubRate => Purpose::Rate,
            Trigger::HubEdit => Purpose::Edit,
        }
    }
}

#[derive(Clone)]
struct Ticket {
    id: String,
    /// `None` only for a voice ticket whose MOG the host could not tell (see `mint_voice`).
    mog_id: Option<String>,
    trigger: Trigger,
    binding: Option<[u8; 32]>,
    /// On the host's monotonic clock (the `now_ms` of each call).
    expires_at_ms: u64,
    /// The wall clock at mint, in epoch ms: a Rate redeem may only change a row that no write
    /// touched after the click (security review M1).
    minted_at_wall_ms: i64,
}

/// The proof of one redeemed ticket. Only [`ActionTickets::redeem`] makes one, so
/// `local_store::exec_ticketed` cannot run without a redeem.
#[derive(Clone, PartialEq, Eq)]
pub struct Redeemed {
    trigger: Trigger,
    minted_at_wall_ms: i64,
}

impl Redeemed {
    pub fn trigger(&self) -> Trigger {
        self.trigger
    }

    pub fn purpose(&self) -> Purpose {
        self.trigger.purpose()
    }

    /// The wall clock at mint, in epoch ms.
    pub fn minted_at_ms(&self) -> i64 {
        self.minted_at_wall_ms
    }

    /// A proof for a unit test of the catalog, with a chosen mint time.
    #[cfg(test)]
    pub(crate) fn for_test(trigger: Trigger, minted_at_wall_ms: i64) -> Self {
        Self {
            trigger,
            minted_at_wall_ms,
        }
    }
}

impl std::fmt::Debug for Redeemed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redeemed")
            .field("trigger", &self.trigger)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Ticket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No id: a ticket id in a log could be replayed.
        f.debug_struct("Ticket")
            .field("trigger", &self.trigger)
            .finish_non_exhaustive()
    }
}

fn wall_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Why a ticket was refused. The host answers `invalid_args` for each one; the reason is for
/// tests only and never crosses IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Unknown,
    Expired,
    WrongPurpose,
    WrongMog,
    /// The write does not hold what the user clicked or said.
    WrongBinding,
}

/// The open tickets of the process.
#[derive(Debug, Default)]
pub struct ActionTickets {
    open: Mutex<VecDeque<Ticket>>,
}

fn hash(v: &Value) -> [u8; 32] {
    // `to_string` of a `Value` is compact JSON with no extra white space.
    Sha256::digest(v.to_string().as_bytes()).into()
}

fn str_of<'a>(p: &'a Value, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Value::as_str)
}

/// A field that must be absent or `null`.
fn absent(p: &Value, key: &str) -> bool {
    p.get(key).is_none_or(Value::is_null)
}

/// The binding array of `trigger` from the write params, or `None` if the params break a rule
/// of the trigger.
fn binding_of(trigger: Trigger, p: &Value) -> Option<Option<Value>> {
    let origin_ok = |want: Option<&str>| match (str_of(p, "origin"), want) {
        (None, None) => absent(p, "origin"),
        (Some(o), None) => o == "pin",
        (Some(o), Some(w)) => o == w,
        (None, Some(_)) => false,
    };
    // In slice 1 the user never types tags on a pin, so a pin carries none (review M2).
    let user_pin = matches!(trigger, Trigger::Ui | Trigger::Voice | Trigger::HubAdd);
    if user_pin && !absent(p, "tags") {
        return None;
    }
    match trigger {
        Trigger::Ui => origin_ok(None).then_some(None),
        Trigger::Voice => {
            if !absent(p, "title") || !absent(p, "body") || !origin_ok(None) {
                return None;
            }
            Some(Some(json!([str_of(p, "text")?])))
        }
        Trigger::HubAdd => {
            if !absent(p, "text") || !origin_ok(Some("added")) {
                return None;
            }
            Some(Some(json!([
                "add",
                str_of(p, "itemType")?,
                str_of(p, "title").unwrap_or(""),
                str_of(p, "body")?,
            ])))
        }
        Trigger::HubRate => Some(Some(json!([str_of(p, "itemId")?]))),
        Trigger::HubEdit => Some(Some(json!([
            "edit",
            str_of(p, "itemId")?,
            str_of(p, "itemType")?,
            str_of(p, "title")?,
            str_of(p, "body")?,
        ]))),
    }
}

/// A new random ticket id (a UUID v4 from the OS random source), or `None` if the OS gives no
/// random bytes: then the host mints no ticket and the user-only write is refused.
pub fn new_ticket_id() -> Option<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).ok()?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Some(format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    ))
}

impl ActionTickets {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<Ticket>> {
        self.open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn mint(
        &self,
        id: String,
        mog_id: Option<&str>,
        trigger: Trigger,
        binding: Option<Value>,
        now_ms: u64,
    ) -> String {
        let mut open = self.lock();
        open.retain(|t| t.expires_at_ms > now_ms);
        while open.len() >= MAX_OPEN {
            open.pop_front();
        }
        open.push_back(Ticket {
            id: id.clone(),
            mog_id: mog_id.map(str::to_owned),
            trigger,
            binding: binding.as_ref().map(hash),
            expires_at_ms: now_ms.saturating_add(TICKET_TTL_MS),
            minted_at_wall_ms: wall_now_ms(),
        });
        id
    }

    /// A card "Remember this" click of `mog_id`. `id` is a new UUID.
    pub fn mint_ui(&self, id: String, mog_id: &str, now_ms: u64) -> String {
        self.mint(id, Some(mog_id), Trigger::Ui, None, now_ms)
    }

    /// The voice command, bound to the remembered `text`. `mog_id` is the MOG of the lease, if
    /// the host can tell it; with `None` the ticket fits a pin of exactly `text` in any MOG.
    pub fn mint_voice(&self, id: String, mog_id: Option<&str>, text: &str, now_ms: u64) -> String {
        self.mint(id, mog_id, Trigger::Voice, Some(json!([text])), now_ms)
    }

    /// A hub "New memory" save.
    pub fn mint_hub_add(
        &self,
        id: String,
        mog_id: &str,
        kind: &str,
        title: &str,
        body: &str,
        now_ms: u64,
    ) -> String {
        let b = json!(["add", kind, title, body]);
        self.mint(id, Some(mog_id), Trigger::HubAdd, Some(b), now_ms)
    }

    /// A hub thumbs up on `item_id`.
    pub fn mint_hub_rate(&self, id: String, mog_id: &str, item_id: &str, now_ms: u64) -> String {
        self.mint(
            id,
            Some(mog_id),
            Trigger::HubRate,
            Some(json!([item_id])),
            now_ms,
        )
    }

    /// A hub edit of `item_id`.
    #[allow(clippy::too_many_arguments)]
    pub fn mint_hub_edit(
        &self,
        id: String,
        mog_id: &str,
        item_id: &str,
        kind: &str,
        title: &str,
        body: &str,
        now_ms: u64,
    ) -> String {
        let b = json!(["edit", item_id, kind, title, body]);
        self.mint(id, Some(mog_id), Trigger::HubEdit, Some(b), now_ms)
    }

    /// Use the ticket `id` for a write of `purpose` with `params` in `mog_id`. The ticket is gone
    /// after this call, also when it is refused, so it can never be tried twice. `now_ms` is on
    /// the host's monotonic clock, as at mint.
    pub fn redeem(
        &self,
        id: &str,
        mog_id: &str,
        purpose: Purpose,
        params: &Value,
        now_ms: u64,
    ) -> Result<Redeemed, Refusal> {
        let t = {
            let mut open = self.lock();
            let at = open
                .iter()
                .position(|t| t.id == id)
                .ok_or(Refusal::Unknown)?;
            open.remove(at).ok_or(Refusal::Unknown)?
        };
        if t.expires_at_ms <= now_ms {
            return Err(Refusal::Expired);
        }
        if t.trigger.purpose() != purpose {
            return Err(Refusal::WrongPurpose);
        }
        if t.mog_id.as_deref().is_some_and(|m| m != mog_id) {
            return Err(Refusal::WrongMog);
        }
        let want = binding_of(t.trigger, params).ok_or(Refusal::WrongBinding)?;
        if want.as_ref().map(hash) != t.binding {
            return Err(Refusal::WrongBinding);
        }
        Ok(Redeemed {
            trigger: t.trigger,
            minted_at_wall_ms: t.minted_at_wall_ms,
        })
    }

    /// The number of open tickets (for tests).
    pub fn open_count(&self) -> usize {
        self.lock().len()
    }
}

/// Words after "remember this" that make the phrase a time, not an order ("remember this week").
const NOT_AN_ORDER: &[&str] = &["week", "month", "year", "time", "morning", "quarter"];

fn strip_word<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let head = s.get(..word.len())?;
    if !head.eq_ignore_ascii_case(word) {
        return None;
    }
    let rest = &s[word.len()..];
    // A word ends at a non-letter.
    if rest.chars().next().is_some_and(char::is_alphanumeric) {
        return None;
    }
    Some(rest)
}

fn skip_sep(s: &str) -> &str {
    s.trim_start_matches(|c: char| {
        c.is_whitespace() || matches!(c, ',' | ':' | ';' | '.' | '-' | '—' | '–')
    })
}

/// The remembered text of the voice command "remember this: <text>", or `None` if `utterance`
/// is not the command or has no text after it. The grammar is the one of `isRememberThis` in
/// `memory-service.ts`: an optional "please", "ok", "okay" or "hey <name>", then "remember this"
/// or "remember that", not followed by a time word. The shared fixture file keeps both sides
/// equal. The text is trimmed, and a final full stop of the transcript goes.
pub fn remember_body(utterance: &str) -> Option<String> {
    let nfkc: String = icu_normalizer::ComposingNormalizerBorrowed::new_nfkc()
        .normalize(utterance)
        .into_owned();
    let mut s = nfkc.trim_start();
    for lead in ["please", "okay", "ok"] {
        if let Some(rest) = strip_word(s, lead) {
            s = skip_sep(rest);
            break;
        }
    }
    if let Some(rest) = strip_word(s, "hey") {
        let rest = rest.trim_start();
        let name_end = rest
            .find(|c: char| !c.is_alphanumeric())
            .unwrap_or(rest.len());
        if name_end > 0 {
            s = skip_sep(&rest[name_end..]);
        }
    }
    let rest = strip_word(s, "remember")?;
    let rest = rest.trim_start();
    let rest = strip_word(rest, "this").or_else(|| strip_word(rest, "that"))?;
    let after = rest.trim_start();
    let next: String = after
        .chars()
        .take_while(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    if NOT_AN_ORDER.contains(&next.as_str()) {
        return None;
    }
    let body = skip_sep(rest).trim();
    let body = body.strip_suffix('.').unwrap_or(body).trim_end();
    (!body.is_empty()).then(|| body.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOG_A: &str = "7f0c2e1a-3d4b-4c5e-9f60-1a2b3c4d5e6f";
    const MOG_B: &str = "0a1b2c3d-0000-4000-8000-000000000002";
    const ITEM: &str = "33333333-0000-4000-8000-000000000003";
    const ITEM_2: &str = "44444444-0000-4000-8000-000000000004";
    const T1: &str = "11111111-0000-4000-8000-000000000001";

    fn pin(text: &str) -> Value {
        json!({ "mogId": MOG_A, "text": text, "pinned": true, "origin": "pin" })
    }

    fn add(kind: &str, title: &str, body: &str) -> Value {
        json!({ "mogId": MOG_A, "itemType": kind, "title": title, "body": body,
                "pinned": true, "origin": "added" })
    }

    fn edit(item: &str, kind: &str, title: &str, body: &str) -> Value {
        json!({ "mogId": MOG_A, "itemId": item, "itemType": kind, "title": title, "body": body })
    }

    #[test]
    fn ac_1_2_a_ui_ticket_works_once() {
        let t = ActionTickets::new();
        t.mint_ui(T1.into(), MOG_A, 1_000);
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Pin, &pin("x"), 2_000)
                .map(|r| r.trigger()),
            Ok(Trigger::Ui)
        );
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Pin, &pin("x"), 2_000),
            Err(Refusal::Unknown)
        );
    }

    #[test]
    fn an_unknown_ticket_is_refused() {
        let t = ActionTickets::new();
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Pin, &pin("x"), 0),
            Err(Refusal::Unknown)
        );
    }

    #[test]
    fn a_ticket_expires_on_the_injected_clock() {
        let t = ActionTickets::new();
        t.mint_ui(T1.into(), MOG_A, 1_000);
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Pin, &pin("x"), 1_000 + TICKET_TTL_MS),
            Err(Refusal::Expired)
        );
        t.mint_ui(T1.into(), MOG_A, 1_000);
        assert!(t
            .redeem(
                T1,
                MOG_A,
                Purpose::Pin,
                &pin("x"),
                1_000 + TICKET_TTL_MS - 1
            )
            .is_ok());
    }

    #[test]
    fn a_ticket_is_bound_to_its_mog_and_is_gone_after_a_refusal() {
        let t = ActionTickets::new();
        t.mint_ui(T1.into(), MOG_A, 0);
        assert_eq!(
            t.redeem(T1, MOG_B, Purpose::Pin, &pin("x"), 1),
            Err(Refusal::WrongMog)
        );
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Pin, &pin("x"), 1),
            Err(Refusal::Unknown)
        );
    }

    #[test]
    fn a_ui_pin_cannot_claim_a_hub_add() {
        let t = ActionTickets::new();
        t.mint_ui(T1.into(), MOG_A, 0);
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Pin, &add("fact", "", "x"), 1),
            Err(Refusal::WrongBinding)
        );
    }

    #[test]
    fn a_voice_ticket_is_bound_to_the_spoken_text() {
        let t = ActionTickets::new();
        t.mint_voice(T1.into(), Some(MOG_A), "the mart refreshes at six", 0);
        assert_eq!(
            t.redeem(
                T1,
                MOG_A,
                Purpose::Pin,
                &pin("the mart refreshes at seven"),
                1
            ),
            Err(Refusal::WrongBinding)
        );
        t.mint_voice(T1.into(), Some(MOG_A), "the mart refreshes at six", 0);
        assert_eq!(
            t.redeem(
                T1,
                MOG_A,
                Purpose::Pin,
                &pin("the mart refreshes at six"),
                1
            )
            .map(|r| r.trigger()),
            Ok(Trigger::Voice)
        );
        // A voice pin has no title or body of its own.
        t.mint_voice(T1.into(), Some(MOG_A), "x", 0);
        let mut p = pin("x");
        p["body"] = json!("other words");
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Pin, &p, 1),
            Err(Refusal::WrongBinding)
        );
        // With no lease MOG the ticket fits any MOG, but only the spoken text.
        t.mint_voice(T1.into(), None, "x", 0);
        assert_eq!(
            t.redeem(T1, MOG_B, Purpose::Pin, &json!({"text": "x"}), 1)
                .map(|r| r.trigger()),
            Ok(Trigger::Voice)
        );
    }

    #[test]
    fn a_hub_add_binds_kind_title_body_and_origin() {
        let t = ActionTickets::new();
        t.mint_hub_add(T1.into(), MOG_A, "fact", "", "Mart at six", 0);
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Pin, &add("fact", "", "Mart at six"), 1)
                .map(|r| r.trigger()),
            Ok(Trigger::HubAdd)
        );
        for p in [
            add("decision", "", "Mart at six"),
            add("fact", "T", "Mart at six"),
            add("fact", "", "Mart at seven"),
            {
                let mut p = add("fact", "", "Mart at six");
                p["origin"] = json!("pin");
                p
            },
            {
                let mut p = add("fact", "", "Mart at six");
                p["text"] = json!("a different summary");
                p
            },
        ] {
            t.mint_hub_add(T1.into(), MOG_A, "fact", "", "Mart at six", 0);
            assert_eq!(
                t.redeem(T1, MOG_A, Purpose::Pin, &p, 1),
                Err(Refusal::WrongBinding),
                "{p}"
            );
        }
        // A missing title binds as "".
        t.mint_hub_add(T1.into(), MOG_A, "fact", "", "b", 0);
        let mut p = add("fact", "", "b");
        p.as_object_mut().map(|o| o.remove("title"));
        assert!(t.redeem(T1, MOG_A, Purpose::Pin, &p, 1).is_ok());
    }

    #[test]
    fn a_rate_ticket_is_bound_to_its_item_and_its_purpose() {
        let t = ActionTickets::new();
        let rate = |item: &str| json!({ "mogId": MOG_A, "itemId": item });
        t.mint_hub_rate(T1.into(), MOG_A, ITEM, 0);
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Rate, &rate(ITEM), 1)
                .map(|r| r.trigger()),
            Ok(Trigger::HubRate)
        );
        t.mint_hub_rate(T1.into(), MOG_A, ITEM, 0);
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Rate, &rate(ITEM_2), 1),
            Err(Refusal::WrongBinding)
        );
        // A rate ticket never pins or edits, and a pin ticket never rates.
        t.mint_hub_rate(T1.into(), MOG_A, ITEM, 0);
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Pin, &pin("x"), 1),
            Err(Refusal::WrongPurpose)
        );
        t.mint_hub_rate(T1.into(), MOG_A, ITEM, 0);
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Edit, &edit(ITEM, "fact", "t", "b"), 1),
            Err(Refusal::WrongPurpose)
        );
        t.mint_ui(T1.into(), MOG_A, 0);
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Rate, &rate(ITEM), 1),
            Err(Refusal::WrongPurpose)
        );
        assert_eq!(t.open_count(), 0);
    }

    #[test]
    fn an_edit_ticket_binds_item_kind_title_and_body() {
        let t = ActionTickets::new();
        t.mint_hub_edit(T1.into(), MOG_A, ITEM, "fact", "T", "B", 0);
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Edit, &edit(ITEM, "fact", "T", "B"), 1)
                .map(|r| r.trigger()),
            Ok(Trigger::HubEdit)
        );
        for p in [
            edit(ITEM_2, "fact", "T", "B"),
            edit(ITEM, "decision", "T", "B"),
            edit(ITEM, "fact", "T2", "B"),
            edit(ITEM, "fact", "T", "B2"),
        ] {
            t.mint_hub_edit(T1.into(), MOG_A, ITEM, "fact", "T", "B", 0);
            assert_eq!(
                t.redeem(T1, MOG_A, Purpose::Edit, &p, 1),
                Err(Refusal::WrongBinding),
                "{p}"
            );
        }
    }

    /// Security review M2: a user pin carries no tags in slice 1.
    #[test]
    fn a_user_pin_ticket_refuses_tags() {
        let t = ActionTickets::new();
        let with_tags = |mut p: Value| {
            p["tags"] = json!(["x"]);
            p
        };
        t.mint_ui(T1.into(), MOG_A, 0);
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Pin, &with_tags(pin("x")), 1),
            Err(Refusal::WrongBinding)
        );
        t.mint_voice(T1.into(), Some(MOG_A), "x", 0);
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Pin, &with_tags(pin("x")), 1),
            Err(Refusal::WrongBinding)
        );
        t.mint_hub_add(T1.into(), MOG_A, "fact", "", "b", 0);
        assert_eq!(
            t.redeem(T1, MOG_A, Purpose::Pin, &with_tags(add("fact", "", "b")), 1),
            Err(Refusal::WrongBinding)
        );
    }

    #[test]
    fn a_redeem_reports_the_mint_time_and_no_debug_shows_the_id() {
        let t = ActionTickets::new();
        let before = wall_now_ms();
        t.mint_ui(T1.into(), MOG_A, 0);
        let r = t.redeem(T1, MOG_A, Purpose::Pin, &pin("x"), 1).unwrap();
        assert!(r.minted_at_ms() >= before && r.minted_at_ms() <= wall_now_ms());
        assert_eq!(r.purpose(), Purpose::Pin);
        t.mint_ui(T1.into(), MOG_A, 0);
        let shown = format!("{r:?} {t:?}");
        assert!(!shown.contains(T1) && !shown.contains(MOG_A), "{shown}");
    }

    #[test]
    fn the_map_keeps_at_most_max_open() {
        let t = ActionTickets::new();
        for i in 0..(MAX_OPEN + 4) {
            t.mint_ui(format!("{i:08}-0000-4000-8000-000000000000"), MOG_A, 0);
        }
        assert_eq!(t.open_count(), MAX_OPEN);
        // The oldest went first.
        assert_eq!(
            t.redeem(
                "00000000-0000-4000-8000-000000000000",
                MOG_A,
                Purpose::Pin,
                &pin("x"),
                1
            ),
            Err(Refusal::Unknown)
        );
    }

    #[test]
    fn ac_1_3_the_voice_body() {
        for (u, want) in [
            (
                "remember this: the mart refreshes at six",
                "the mart refreshes at six",
            ),
            (
                "Remember this, the mart refreshes at six.",
                "the mart refreshes at six",
            ),
            (
                "Please remember that Q3 churn uses the new formula",
                "Q3 churn uses the new formula",
            ),
            (
                "hey Bottle, remember this — EUR for amounts",
                "EUR for amounts",
            ),
            ("ＲＥＭＥＭＢＥＲ this: x", "x"),
        ] {
            assert_eq!(remember_body(u).as_deref(), Some(want), "{u}");
        }
    }

    #[test]
    fn ac_1_4_not_the_command() {
        for u in [
            "remember this week",
            "Remember this month, sales were up",
            "remember this",
            "remember this.",
            "do you remember this: x",
            "remembering this: x",
            "remember thistle: x",
            "remember these: x",
            "what did I ask Genie to remember",
            "",
        ] {
            assert_eq!(remember_body(u), None, "{u}");
        }
    }
}
