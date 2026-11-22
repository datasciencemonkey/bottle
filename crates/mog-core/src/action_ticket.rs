//! Host-minted user-action tickets (#574, MEM-28b; ADR 0113).
//!
//! A "Remember this" pin needs proof of a user action, and the brain cannot give that proof: it
//! is code, not a user. So the host mints a ticket on the user's own action and the brain passes
//! it back with the pin:
//! - `Ui`: the card window of a MOG sends the click `remember_this`. Only a view window can send
//!   `view_emit`; the brain cannot.
//! - `Voice`: the host's own transcript of the user's speech holds "remember this: <text>"
//!   ([`remember_body`]). The ticket binds the SHA-256 of `<text>`, so the pin must hold exactly
//!   the words the user said.
//! - `HubRate`: the hub sends a thumbs up (`memory.rate`) on one item. The ticket binds the item
//!   id. With it, and only with it, the host lets the thumbs up release a `quarantined` item to
//!   `active` (owner ruling 2026-11-21, "Quarantine, thumbs up releases"; SEC-24b amended).
//!
//! Each ticket has a [`Purpose`], so a pin ticket can never release an item and a release ticket
//! can never pin.
//!
//! A ticket is single-use, bound to one MOG, and expires after [`TICKET_TTL_MS`]. [`ActionTickets::redeem`]
//! takes it out of the map before the write runs, so a replay is refused. The map holds at
//! most [`MAX_OPEN`] tickets; a new one drops the oldest. A ticket holds no text, only a hash.

use std::collections::VecDeque;
use std::sync::Mutex;

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
    /// A thumbs up on one item in the hub.
    HubRate,
}

/// What a ticket allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// A "remember this" pin (`mem.item.write` with `pinned: true`).
    Pin,
    /// A thumbs up that may release a quarantined item (`mem.item.rate`).
    Rate,
}

impl Trigger {
    fn purpose(self) -> Purpose {
        match self {
            Trigger::Ui | Trigger::Voice => Purpose::Pin,
            Trigger::HubRate => Purpose::Rate,
        }
    }
}

#[derive(Debug, Clone)]
struct Ticket {
    id: String,
    mog_id: String,
    trigger: Trigger,
    text_sha256: Option<[u8; 32]>,
    expires_at_ms: u64,
}

/// Why a ticket was refused. The host answers `invalid_args` for each one; the reason is for
/// tests only and never crosses IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Unknown,
    Expired,
    WrongMog,
    WrongText,
    WrongPurpose,
}

/// The open tickets of the process.
#[derive(Debug, Default)]
pub struct ActionTickets {
    open: Mutex<VecDeque<Ticket>>,
}

fn sha256(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
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
        mog_id: &str,
        trigger: Trigger,
        text: Option<&str>,
        now_ms: u64,
    ) -> String {
        let mut open = self.lock();
        open.retain(|t| t.expires_at_ms > now_ms);
        while open.len() >= MAX_OPEN {
            open.pop_front();
        }
        open.push_back(Ticket {
            id: id.clone(),
            mog_id: mog_id.to_owned(),
            trigger,
            text_sha256: text.map(sha256),
            expires_at_ms: now_ms.saturating_add(TICKET_TTL_MS),
        });
        id
    }

    /// A ticket for a card click of `mog_id`. `id` is a new UUID.
    pub fn mint_ui(&self, id: String, mog_id: &str, now_ms: u64) -> String {
        self.mint(id, mog_id, Trigger::Ui, None, now_ms)
    }

    /// A ticket for the voice command of `mog_id`, bound to the remembered `text`.
    pub fn mint_voice(&self, id: String, mog_id: &str, text: &str, now_ms: u64) -> String {
        self.mint(id, mog_id, Trigger::Voice, Some(text), now_ms)
    }

    /// A ticket for a hub thumbs up on the item `item_id` of `mog_id`.
    pub fn mint_rate(&self, id: String, mog_id: &str, item_id: &str, now_ms: u64) -> String {
        self.mint(id, mog_id, Trigger::HubRate, Some(item_id), now_ms)
    }

    /// Use the ticket `id` for a pin of `text` in `mog_id`. The ticket is gone after this call,
    /// also when it is refused, so it can never be tried twice.
    pub fn redeem(
        &self,
        id: &str,
        mog_id: &str,
        text: &str,
        now_ms: u64,
    ) -> Result<Trigger, Refusal> {
        self.redeem_for(Purpose::Pin, id, mog_id, text, now_ms)
    }

    /// Use the ticket `id` for a thumbs up on `item_id` in `mog_id`.
    pub fn redeem_rate(
        &self,
        id: &str,
        mog_id: &str,
        item_id: &str,
        now_ms: u64,
    ) -> Result<Trigger, Refusal> {
        self.redeem_for(Purpose::Rate, id, mog_id, item_id, now_ms)
    }

    fn redeem_for(
        &self,
        purpose: Purpose,
        id: &str,
        mog_id: &str,
        bound: &str,
        now_ms: u64,
    ) -> Result<Trigger, Refusal> {
        let mut open = self.lock();
        let at = open
            .iter()
            .position(|t| t.id == id)
            .ok_or(Refusal::Unknown)?;
        let t = open.remove(at).ok_or(Refusal::Unknown)?;
        drop(open);
        if t.expires_at_ms <= now_ms {
            return Err(Refusal::Expired);
        }
        if t.trigger.purpose() != purpose {
            return Err(Refusal::WrongPurpose);
        }
        if t.mog_id != mog_id {
            return Err(Refusal::WrongMog);
        }
        if let Some(h) = t.text_sha256 {
            if h != sha256(bound) {
                return Err(Refusal::WrongText);
            }
        }
        Ok(t.trigger)
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
    const T1: &str = "11111111-0000-4000-8000-000000000001";
    const T2: &str = "22222222-0000-4000-8000-000000000002";

    #[test]
    fn ac_1_2_a_ticket_works_once() {
        let t = ActionTickets::new();
        let id = t.mint_ui(T1.into(), MOG_A, 1_000);
        assert_eq!(t.redeem(&id, MOG_A, "any text", 2_000), Ok(Trigger::Ui));
        assert_eq!(
            t.redeem(&id, MOG_A, "any text", 2_000),
            Err(Refusal::Unknown)
        );
    }

    #[test]
    fn an_unknown_ticket_is_refused() {
        let t = ActionTickets::new();
        assert_eq!(t.redeem(T1, MOG_A, "x", 0), Err(Refusal::Unknown));
    }

    #[test]
    fn a_ticket_expires() {
        let t = ActionTickets::new();
        t.mint_ui(T1.into(), MOG_A, 1_000);
        assert_eq!(
            t.redeem(T1, MOG_A, "x", 1_000 + TICKET_TTL_MS),
            Err(Refusal::Expired)
        );
    }

    #[test]
    fn a_ticket_is_bound_to_its_mog_and_is_gone_after_a_refusal() {
        let t = ActionTickets::new();
        t.mint_ui(T1.into(), MOG_A, 0);
        assert_eq!(t.redeem(T1, MOG_B, "x", 1), Err(Refusal::WrongMog));
        assert_eq!(t.redeem(T1, MOG_A, "x", 1), Err(Refusal::Unknown));
    }

    #[test]
    fn a_voice_ticket_is_bound_to_the_spoken_text() {
        let t = ActionTickets::new();
        t.mint_voice(T1.into(), MOG_A, "the mart refreshes at six", 0);
        assert_eq!(
            t.redeem(T1, MOG_A, "the mart refreshes at seven", 1),
            Err(Refusal::WrongText)
        );
        t.mint_voice(T2.into(), MOG_A, "the mart refreshes at six", 0);
        assert_eq!(
            t.redeem(T2, MOG_A, "the mart refreshes at six", 1),
            Ok(Trigger::Voice)
        );
    }

    #[test]
    fn a_rate_ticket_is_bound_to_its_item_and_its_purpose() {
        let t = ActionTickets::new();
        let item = "33333333-0000-4000-8000-000000000003";
        t.mint_rate(T1.into(), MOG_A, item, 0);
        assert_eq!(t.redeem_rate(T1, MOG_A, item, 1), Ok(Trigger::HubRate));
        t.mint_rate(T1.into(), MOG_A, item, 0);
        assert_eq!(
            t.redeem_rate(T1, MOG_A, "44444444-0000-4000-8000-000000000004", 1),
            Err(Refusal::WrongText)
        );
        // A rate ticket never pins, and a pin ticket never releases.
        t.mint_rate(T1.into(), MOG_A, item, 0);
        assert_eq!(t.redeem(T1, MOG_A, item, 1), Err(Refusal::WrongPurpose));
        t.mint_ui(T2.into(), MOG_A, 0);
        assert_eq!(
            t.redeem_rate(T2, MOG_A, item, 1),
            Err(Refusal::WrongPurpose)
        );
        assert_eq!(t.open_count(), 0);
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
            t.redeem("00000000-0000-4000-8000-000000000000", MOG_A, "x", 1),
            Err(Refusal::Unknown)
        );
    }

    #[test]
    fn ac_1_3_the_voice_body() {
        assert_eq!(
            remember_body("remember this: the mart refreshes at six").as_deref(),
            Some("the mart refreshes at six")
        );
        assert_eq!(
            remember_body("Remember this, the mart refreshes at six.").as_deref(),
            Some("the mart refreshes at six")
        );
        assert_eq!(
            remember_body("Please remember that Q3 churn uses the new formula").as_deref(),
            Some("Q3 churn uses the new formula")
        );
        assert_eq!(
            remember_body("hey Bottle, remember this — EUR for amounts").as_deref(),
            Some("EUR for amounts")
        );
        assert_eq!(
            remember_body("ＲＥＭＥＭＢＥＲ this: x").as_deref(),
            Some("x")
        );
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
