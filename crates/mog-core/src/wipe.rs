//! R14 WipeService (section 4.3; #124, ADR 0087).
//!
//! Responsibilities: Deletes user content only when the user selects Wipe in Settings (older than 30 d, older than 90 d, or everything); (v1) applies the same selection to Lakebase on the next connection. Also runs the content-free security expiries: the audio deletion deadline, log rotation and consent records (U9).
//!
//! - [`WipeService::wipe`] is the only path that deletes user content by age (MEM-33a, U9). Its
//!   one caller is the `mem_wipe` command (contract change #525), which the brain sends only from
//!   the confirm click of the hub Wipe dialog. There is no timer and no background job.
//! - [`LocalWipeService::wipe`] runs in one R10 transaction for every MOG of the device. It reads
//!   the MOG ids (the device-wide [`catalog::WIPE_MOG_LIST`]), then for each MOG it reads the Genie
//!   conversations of the rows it deletes and runs the MOG-scoped group `mem.wipe.before` or
//!   `mem.wipe.everything` (MEM-01). FTS5 and vec0 rows go with their rows (0001 triggers), so
//!   recall no longer finds a wiped text.
//! - [`WipeService::run_security_expiries`] runs the content-free expiries of 9.2: the consent
//!   records (SEC-15b, [`ConsentRecords::purge_expired`]) and the action log (ADR 0059,
//!   [`AuditLog::rotate`]). They never touch user content. The host runs them once at start and
//!   after each Wipe. TODO(#125): the audio journal deadline registers its expiry here.
//!   TODO(#517): the local log file rotation registers its expiry here.
//! - TODO(#216): (v1) apply the same selection to Lakebase on the next connection (MEM-33b).

use std::sync::Arc;

use serde_json::{json, Value};

use crate::audit::{AuditLog, AuditStore};
use crate::clock::WallClock;
use crate::consent::ConsentRecords;
use crate::error::{CoreError, CoreResult};
use crate::ipc::MemWipeArgs;
use crate::local_store::catalog;
use crate::local_store::SqlCipherLocalStore;

const ID: &str = "R14";
const DAY_MS: i64 = 86_400_000;

/// The Wipe choice that the user selects in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WipeScope {
    /// User content older than 30 days.
    OlderThan30Days,
    /// User content older than 90 days.
    OlderThan90Days,
    /// All user content.
    Everything,
}

impl WipeScope {
    /// The wire name of the `mem_wipe` command (#525).
    pub fn wire(self) -> &'static str {
        match self {
            WipeScope::OlderThan30Days => "older_than_30d",
            WipeScope::OlderThan90Days => "older_than_90d",
            WipeScope::Everything => "everything",
        }
    }

    /// The scope with this exact wire name.
    pub fn parse(s: &str) -> Option<Self> {
        [
            WipeScope::OlderThan30Days,
            WipeScope::OlderThan90Days,
            WipeScope::Everything,
        ]
        .into_iter()
        .find(|w| w.wire() == s)
    }

    /// The cutoff in epoch ms: a row strictly older goes. `None` for everything.
    pub fn cutoff_ms(self, now_ms: i64) -> Option<i64> {
        match self {
            WipeScope::OlderThan30Days => Some(now_ms.saturating_sub(30 * DAY_MS)),
            WipeScope::OlderThan90Days => Some(now_ms.saturating_sub(90 * DAY_MS)),
            WipeScope::Everything => None,
        }
    }
}

/// One Genie conversation of the wiped exchanges that no kept exchange uses (9.2, G-49). Ids
/// only: the brain deletes it in Genie only if the user also selected "Also delete Genie
/// conversations".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WipeConversation {
    /// The MOG of the exchange.
    pub mog_id: String,
    /// The backend kind of the exchange, as stored (data, not a branch).
    pub backend_kind: String,
    /// The profile that made the conversation; a delete goes to its workspace only.
    pub profile: String,
    /// The workspace host.
    pub host: String,
    /// The numeric workspace id, if known.
    pub workspace_id: Option<i64>,
    /// The Genie space, if the backend has one.
    pub space_id: Option<String>,
    /// The conversation id.
    pub conversation_id: String,
}

/// What a Wipe deleted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WipeReport {
    /// The local rows deleted: items, embed jobs, exchanges, utterances and sessions.
    pub local: u64,
    /// The Genie conversations of the deleted exchanges, by MOG then conversation id.
    pub conversations: Vec<WipeConversation>,
}

/// What the security expiries deleted, in the order they ran.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExpiryReport {
    /// `(name, rows deleted)` for each expiry.
    pub expired: Vec<(&'static str, usize)>,
}

/// One content-free security expiry of 9.2. It must never delete user content (U9).
pub trait SecurityExpiry: Send + Sync {
    /// A fixed name for the report.
    fn name(&self) -> &'static str;
    /// Delete the expired rows at `now_ms`; answer how many went.
    fn expire(&self, now_ms: i64) -> CoreResult<usize>;
}

/// SEC-15b: consent records, 13 months (admin 12-84). The records keep their own clock.
impl SecurityExpiry for ConsentRecords {
    fn name(&self) -> &'static str {
        "consent_records"
    }
    fn expire(&self, _now_ms: i64) -> CoreResult<usize> {
        self.purge_expired()
    }
}

/// 9.2 "Main agent action log": content-free, 30 d (admin 7-90), rotation (ADR 0059).
impl<S: AuditStore> SecurityExpiry for AuditLog<S> {
    fn name(&self) -> &'static str {
        "action_log"
    }
    fn expire(&self, now_ms: i64) -> CoreResult<usize> {
        self.rotate(u64::try_from(now_ms).unwrap_or(0))
    }
}

/// Deletes user content only on a user selection, and runs the content-free expiries.
pub trait WipeService: Send + Sync {
    /// Delete user content for the scope that the user selected in Settings.
    fn wipe(&self, scope: WipeScope) -> CoreResult<WipeReport>;
    /// Run the content-free security expiries (audio deadline, log rotation, consent records).
    fn run_security_expiries(&self) -> CoreResult<ExpiryReport>;
}

/// The R14 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubWipeService;

impl WipeService for StubWipeService {
    fn wipe(&self, _scope: WipeScope) -> CoreResult<WipeReport> {
        Err(CoreError::NotImplemented { component: ID })
    }

    fn run_security_expiries(&self) -> CoreResult<ExpiryReport> {
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// The agent session host, as Wipe sees it (slice 2a, plan A3.7): every Wipe scope ends the
/// session, kills the Genie Code process and deletes GENIE_HOME.
pub trait AgentWipeHook: Send + Sync {
    /// End the session and wait until GENIE_HOME is gone. It never fails: a Wipe goes on.
    fn end_for_wipe(&self);
}

/// A hook that is bound after the Wipe service is built (the host builds the Wipe service
/// before the session host). Until it is bound, it does nothing.
#[derive(Default)]
pub struct AgentWipeSlot {
    hook: std::sync::OnceLock<Arc<dyn AgentWipeHook>>,
}

impl AgentWipeSlot {
    /// Bind the session host. The first call wins.
    pub fn bind(&self, hook: Arc<dyn AgentWipeHook>) {
        let _ = self.hook.set(hook);
    }
}

impl AgentWipeHook for AgentWipeSlot {
    fn end_for_wipe(&self) {
        if let Some(h) = self.hook.get() {
            h.end_for_wipe();
        }
    }
}

/// R14 on the R10 local store.
pub struct LocalWipeService {
    store: Arc<SqlCipherLocalStore>,
    clock: Arc<dyn WallClock>,
    expiries: Vec<Arc<dyn SecurityExpiry>>,
    agent: Option<Arc<dyn AgentWipeHook>>,
}

impl LocalWipeService {
    /// A WipeService on `store` and `clock`, with no expiry yet.
    pub fn new(store: Arc<SqlCipherLocalStore>, clock: Arc<dyn WallClock>) -> Self {
        Self {
            store,
            clock,
            expiries: Vec::new(),
            agent: None,
        }
    }

    /// End the agent session on every Wipe (slice 2a).
    pub fn with_agent_hook(mut self, h: Arc<dyn AgentWipeHook>) -> Self {
        self.agent = Some(h);
        self
    }

    /// Add a content-free security expiry. They run in the order they were added.
    pub fn with_expiry(mut self, e: Arc<dyn SecurityExpiry>) -> Self {
        self.expiries.push(e);
        self
    }
}

fn changes(rows: &[Value]) -> u64 {
    rows.iter().filter_map(|r| r["changes"].as_u64()).sum()
}

fn text(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn conversation_of(mog_id: &str, r: &Value) -> Option<WipeConversation> {
    Some(WipeConversation {
        mog_id: mog_id.to_owned(),
        backend_kind: text(&r["backendKind"])?,
        profile: text(&r["profile"])?,
        host: text(&r["host"])?,
        workspace_id: r["workspaceId"].as_i64(),
        space_id: text(&r["spaceId"]),
        conversation_id: text(&r["conversationId"])?,
    })
}

impl WipeService for LocalWipeService {
    fn wipe(&self, scope: WipeScope) -> CoreResult<WipeReport> {
        // Plan A3.7: each scope ends the agent session first. The transcript of a session lives
        // only in memory and in GENIE_HOME, so no scope may leave it.
        if let Some(h) = &self.agent {
            h.end_for_wipe();
        }
        let cutoff = scope.cutoff_ms(self.clock.now_ms());
        self.store.core_transaction(|tx| {
            let mut report = WipeReport::default();
            let mogs: Vec<String> = tx
                .device_query(&catalog::WIPE_MOG_LIST)?
                .iter()
                .filter_map(|r| text(&r["mog_id"]))
                .collect();
            for mog in &mogs {
                let (read, group, params) = match cutoff {
                    Some(before) => (
                        "mem.wipe.conversations_before",
                        "mem.wipe.before",
                        json!({ "mogId": mog, "before": before.max(0) }),
                    ),
                    None => (
                        "mem.forget.mog_conversations",
                        "mem.wipe.everything",
                        json!({ "mogId": mog }),
                    ),
                };
                // Read the conversations first: the delete removes the rows that hold them.
                for r in tx.run(read, &params)? {
                    report.conversations.extend(conversation_of(mog, &r));
                }
                report.local += changes(&tx.group(group, &params)?);
            }
            Ok(report)
        })
    }

    fn run_security_expiries(&self) -> CoreResult<ExpiryReport> {
        let now = self.clock.now_ms();
        let mut report = ExpiryReport::default();
        let mut first_err = None;
        // Each expiry runs even if an earlier one fails.
        for e in &self.expiries {
            match e.expire(now) {
                Ok(n) => report.expired.push((e.name(), n)),
                Err(err) => {
                    first_err.get_or_insert(err);
                }
            }
        }
        match first_err {
            Some(err) => Err(err),
            None => Ok(report),
        }
    }
}

/// The R14 side of the `mem_wipe` command (contract change #525). It parses `{ scope }`, runs the
/// Wipe, then the security expiries, and answers `{ local, conversations }` with ids only. Every
/// refusal is a fixed IPC code with no value in it: `invalid_args` for bad args,
/// `not_implemented` for a service that is not there and `host_failed` for a store failure.
/// An expiry failure after the Wipe does not undo the Wipe or hide its answer.
/// The app host routes `mem_wipe` here (`src-tauri/src/ipc.rs`, #525).
pub fn mem_wipe(service: &dyn WipeService, args: Value) -> CoreResult<Value> {
    let refused = |code: &'static str| CoreError::IpcRefused { code };
    let a: MemWipeArgs = serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
    let scope = WipeScope::parse(&a.scope).ok_or(refused("invalid_args"))?;
    let report = service.wipe(scope).map_err(|e| match e {
        CoreError::NotImplemented { .. } => refused("not_implemented"),
        _ => refused("host_failed"),
    })?;
    let _ = service.run_security_expiries();
    let conversations: Vec<Value> = report
        .conversations
        .iter()
        .map(|c| {
            let mut o = json!({
                "mogId": c.mog_id,
                "backendKind": c.backend_kind,
                "profile": c.profile,
                "host": c.host,
                "conversationId": c.conversation_id,
            });
            if let Some(ws) = c.workspace_id {
                o["workspaceId"] = json!(ws);
            }
            if let Some(space) = &c.space_id {
                o["spaceId"] = json!(space);
            }
            o
        })
        .collect();
    Ok(json!({ "local": report.local, "conversations": conversations }))
}

#[cfg(test)]
mod tests;
