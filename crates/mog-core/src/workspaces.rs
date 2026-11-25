//! One MOG with workspaces (ADR 0117): the `workspaces.json` store and the `workspace_*` commands.
//!
//! - The file lives in the app config dir. It is private (0600), written atomically and holds
//!   no secret: the home profile, the lamp config id, one entry per switched profile and the
//!   agent settings. The old MOG configs stay as they are and keep each workspace's name and
//!   colour (no `MogConfig` change).
//! - When the file is missing, [`WorkspaceStore`] migrates it from the [`MogConfigStore`]: one
//!   entry per `binding.home`, all available. The home is set only when exactly one profile
//!   exists. Bottle never picks a home for the user (ADR 0117, FLAG 5).
//! - A file that cannot be read or parsed is an error. It is never deleted or rewritten.
//! - `workspace_list` renders a stored entry whose profile does not answer in the current CLI
//!   read as absent (owner review 2026-11-24: `databricks auth logout --delete` left a dead row
//!   behind), and a vanished home as `home: None` (so the hub falls back to its normal "Choose
//!   a home workspace" flow). This is cosmetic only, for one render: the file on disk, its
//!   entries, `mog_ids`, `available` and the stored `home` (and so `lamp_mog_id`, which this
//!   never touches) are left exactly as they were. A profile that comes back (or an empty or
//!   failed CLI read that was only a transient glitch, never proof the profile is gone) shows
//!   its row again with its old links, with nothing to restore.
//! - [`WorkspaceGate`] is the allow-list of R8 (FLAG 7): `genie_ask` goes only to the home
//!   profile and to an available profile.
//!
//! No token, question or answer is handled here. The sign-in probe lends a token to an empty
//! closure through the R6 seam, so the token never leaves R6.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{CoreError, CoreResult};
use crate::ipc::{
    WorkspaceListArgs, WorkspaceMakeHomeArgs, WorkspaceSetAvailableArgs, WorkspaceSettingsWriteArgs,
};
use crate::ipc_gateway::GatewayComponents;
use crate::mog_config::MogConfigStore;
use crate::profile_service::ProfileService;
use crate::token_broker::{CliTokenBroker, TokenBroker};
use crate::types::{
    default_clean_dictation, AgentSettings, GenieCallBudget, MogId, OrchestratorModel, Profile,
    ProfileName, TokenState, WorkspaceList, WorkspaceRow, WorkspaceSignIn,
};

/// The file name in the app config dir.
pub const FILE_NAME: &str = "workspaces.json";
/// The schema version of the file.
pub const SCHEMA_VERSION: u32 = 1;
/// The default D9 orchestrator model (ADR 0122).
pub const DEFAULT_ENDPOINT: &str = crate::model_gateway::DEFAULT_ORCHESTRATOR_MODEL;
/// The sign-in probes that run at once, across all calls.
pub const PROBE_PARALLEL: usize = 4;
/// One `workspace_list` call waits at most this long for its probes.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// The largest file that is read.
const MAX_FILE_BYTES: usize = 256 * 1024;

fn refused(code: &'static str) -> CoreError {
    CoreError::IpcRefused { code }
}

fn storage_failed() -> CoreError {
    refused("workspace_storage_failed")
}

/// The allow-list of R8 (ADR 0117, FLAG 7). `McpCallComponent` asks it before any `genie_ask`.
pub trait WorkspaceGate: Send + Sync + std::fmt::Debug {
    /// True if `genie_ask` may go to `profile`: it is the home or it is switched on. It fails
    /// closed: a store that cannot be read allows nothing.
    fn allows(&self, profile: &str) -> bool;
    /// The workspace URL (`https://host`) of a supported `profile`, for the link trust check.
    /// `None` when unknown: no link is then trusted.
    fn workspace_url(&self, _profile: &str) -> Option<String> {
        None
    }
}

/// T7 as the store sees it: the profile list.
pub trait ProfileSource: Send + Sync {
    fn profiles(&self, refresh: bool) -> CoreResult<Vec<Profile>>;
}

impl ProfileSource for ProfileService {
    fn profiles(&self, refresh: bool) -> CoreResult<Vec<Profile>> {
        self.list(refresh)
    }
}

/// R6 as the store sees it: the cached token state and a probe. Neither returns a token.
pub trait SignInSource: Send + Sync {
    /// The cached state of `profile`. It is `Unknown` until the first token use.
    fn state(&self, profile: &str) -> TokenState;
    /// Ask for a token of `profile` for `host` and drop it. `Err` is the R6 failure.
    fn probe(&self, profile: &str, host: &str) -> CoreResult<()>;
}

impl SignInSource for CliTokenBroker {
    fn state(&self, profile: &str) -> TokenState {
        self.token_state(profile)
    }

    fn probe(&self, profile: &str, host: &str) -> CoreResult<()> {
        let bare = reqwest::Url::parse(host.trim())
            .ok()
            .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
            .ok_or(CoreError::Unavailable {
                component: "workspaces",
                reason: "profile_host_invalid",
            })?;
        self.with_token(profile, &bare, &mut |_| {}).map(|_| ())
    }
}

/// True for an R6 failure that means "sign in again".
fn is_auth_error(e: &CoreError) -> bool {
    matches!(
        e,
        CoreError::Unavailable {
            reason: "login_required" | "auth_expired" | "token_unusable",
            ..
        }
    )
}

/// The longest model alias, a `-N` suffix included. It matches the `workspace` limit of `ask_genie`.
const ALIAS_MAX_CHARS: usize = 32;

/// The model alias of a profile name: lower case, `[a-z0-9-]` only, at most `ALIAS_MAX_CHARS` (32) characters.
fn alias_slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let t = out.trim_matches('-');
    let t: String = t.chars().take(ALIAS_MAX_CHARS).collect();
    let t = t.trim_matches('-').to_owned();
    if t.is_empty() {
        "ws".to_owned()
    } else {
        t
    }
}

impl crate::agent_run::orchestrator::WorkspaceView for WorkspaceStore {
    fn snapshot(&self) -> CoreResult<crate::agent_run::orchestrator::Snapshot> {
        self.catalog_snapshot()
    }
}

// ---------- the file ----------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    profile: ProfileName,
    mog_ids: Vec<MogId>,
    available: bool,
}

fn default_model() -> OrchestratorModel {
    OrchestratorModel::try_from(DEFAULT_ENDPOINT.to_owned())
        .expect("the default orchestrator model is a valid id")
}

/// Reads any stored string: a valid `system.ai.` id stays, anything else becomes the default.
fn migrate_model<'de, D: serde::Deserializer<'de>>(d: D) -> Result<OrchestratorModel, D::Error> {
    let s = String::deserialize(d)?;
    Ok(OrchestratorModel::try_from(s).unwrap_or_else(|_| default_model()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentData {
    genie_call_budget: GenieCallBudget,
    /// ADR 0122: the orchestrator model. A stored value that is not a `system.ai.` id (the old
    /// D9 default `databricks-claude-sonnet-5-5`) reads as the default.
    #[serde(deserialize_with = "migrate_model")]
    endpoint: OrchestratorModel,
    #[serde(default = "default_clean_dictation")]
    clean_dictation: bool,
    /// ADR 0123: dictation goes to the home AI Gateway. A file from before reads as false.
    #[serde(default)]
    cloud_dictation: bool,
    /// ADR 0122: the old per-profile run setting. Read so that an old file still loads; never
    /// kept and never written.
    #[serde(default, skip_serializing, rename = "runProfile")]
    _legacy_run_profile: Option<serde_json::Value>,
    /// D1 "Combine across workspaces" is removed (ADR 0122 amendment, owner decision
    /// 2026-11-24): cross-workspace data is always on. Read so that a file from before the
    /// removal still loads; never kept and never written.
    #[serde(default, skip_serializing, rename = "crossWorkspaceData")]
    _legacy_cross_workspace_data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Data {
    schema_version: u32,
    lamp_mog_id: Option<MogId>,
    home: Option<ProfileName>,
    workspaces: Vec<Entry>,
    agent: AgentData,
}

impl Data {
    fn valid(&self) -> bool {
        // The budget and the endpoint name are checked when the file is parsed.
        self.schema_version == SCHEMA_VERSION
    }

    fn entry(&self, profile: &str) -> Option<&Entry> {
        self.workspaces.iter().find(|e| e.profile == profile)
    }

    fn entry_mut(&mut self, profile: &str) -> &mut Entry {
        if let Some(i) = self.workspaces.iter().position(|e| e.profile == profile) {
            return &mut self.workspaces[i];
        }
        self.workspaces.push(Entry {
            profile: profile.to_owned(),
            mog_ids: Vec::new(),
            available: false,
        });
        self.workspaces.last_mut().expect("just pushed")
    }

    fn allows(&self, profile: &str) -> bool {
        self.home.as_deref() == Some(profile) || self.entry(profile).is_some_and(|e| e.available)
    }
}

/// Write `bytes` to a new private temp file in `dir`, flush it, then rename it over `path`.
fn atomic_write(dir: &Path, path: &Path, bytes: &[u8]) -> CoreResult<()> {
    let mut db = fs::DirBuilder::new();
    db.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        db.mode(0o700);
    }
    db.create(dir).map_err(|_| storage_failed())?;
    let mut nonce = [0u8; 8];
    getrandom::fill(&mut nonce).map_err(|_| storage_failed())?;
    let suffix: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    let tmp = dir.join(format!(".{FILE_NAME}.tmp-{suffix}"));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = opts
        .open(&tmp)
        .and_then(|mut f| {
            f.write_all(bytes)?;
            f.sync_all()
        })
        .and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        // Only the temp file that this call made.
        let _ = fs::remove_file(&tmp);
        return Err(storage_failed());
    }
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// A counting semaphore with a deadline: the slots of the sign-in probes.
struct Slots {
    free: Mutex<usize>,
    freed: std::sync::Condvar,
}

impl Slots {
    fn new(n: usize) -> Arc<Self> {
        Arc::new(Self {
            free: Mutex::new(n),
            freed: std::sync::Condvar::new(),
        })
    }

    /// Take a slot, or give up at `deadline`.
    fn acquire(&self, deadline: Instant) -> bool {
        let mut free = self.free.lock().unwrap_or_else(|p| p.into_inner());
        while *free == 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            free = self
                .freed
                .wait_timeout(free, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        *free -= 1;
        true
    }
}

/// Gives the slot back when the probe thread ends, on every path.
struct SlotGuard(Arc<Slots>);

impl Drop for SlotGuard {
    fn drop(&mut self) {
        *self.0.free.lock().unwrap_or_else(|p| p.into_inner()) += 1;
        self.0.freed.notify_one();
    }
}

// ---------- the store ----------

/// The `workspaces.json` store and the `workspace_*` component.
pub struct WorkspaceStore {
    path: PathBuf,
    configs: Arc<MogConfigStore>,
    profiles: Arc<dyn ProfileSource>,
    /// R6, bound after R8 is built (R8 needs this store as its gate first).
    sign_in: OnceLock<Arc<dyn SignInSource>>,
    probe_timeout: Duration,
    slots: Arc<Slots>,
    /// One read-modify-write at a time.
    lock: Mutex<()>,
    /// The parsed file, so the home, the model and the catalog reads of each call do not read
    /// the disk again. Only this store writes the file: `save` empties the cache.
    cache: Mutex<Option<Data>>,
    /// A copy of `agent.cloud_dictation` (ADR 0123), kept by [`Self::load`] and
    /// [`Self::settings_write`], so a key-down reads one atomic instead of the file-backed cache
    /// (no lock, no clone, no disk).
    cloud_dictation_flag: AtomicBool,
}

impl std::fmt::Debug for WorkspaceStore {
    /// Shows nothing: no profile, host or id.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceStore").finish_non_exhaustive()
    }
}

impl WorkspaceStore {
    /// A store of `<dir>/workspaces.json` over the MOG `configs` and the T7 `profiles`.
    pub fn new(
        dir: impl AsRef<Path>,
        configs: Arc<MogConfigStore>,
        profiles: Arc<dyn ProfileSource>,
    ) -> Self {
        Self {
            path: dir.as_ref().join(FILE_NAME),
            configs,
            profiles,
            sign_in: OnceLock::new(),
            probe_timeout: PROBE_TIMEOUT,
            slots: Slots::new(PROBE_PARALLEL),
            lock: Mutex::new(()),
            cache: Mutex::new(None),
            cloud_dictation_flag: AtomicBool::new(false),
        }
    }

    /// Set the time that one sign-in probe waits (tests).
    pub fn with_probe_timeout(mut self, t: Duration) -> Self {
        self.probe_timeout = t;
        self
    }

    /// Bind R6. The first call wins.
    pub fn bind_sign_in(&self, source: Arc<dyn SignInSource>) {
        let _ = self.sign_in.set(source);
    }

    fn dir(&self) -> &Path {
        self.path.parent().unwrap_or_else(|| Path::new("."))
    }

    /// Read the file, or migrate it from the MOG configs when it is missing. A file that is
    /// there but bad is an error and stays on disk. The caller holds `lock`.
    fn load(&self) -> CoreResult<Data> {
        if let Some(d) = self.cache.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            self.cloud_dictation_flag
                .store(d.agent.cloud_dictation, Ordering::SeqCst);
            return Ok(d);
        }
        let data = self.load_from_disk()?;
        *self.cache.lock().unwrap_or_else(|p| p.into_inner()) = Some(data.clone());
        self.cloud_dictation_flag
            .store(data.agent.cloud_dictation, Ordering::SeqCst);
        Ok(data)
    }

    fn load_from_disk(&self) -> CoreResult<Data> {
        let file = match fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let data = self.migrate()?;
                self.save(&data)?;
                return Ok(data);
            }
            Err(_) => return Err(storage_failed()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| storage_failed())?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(storage_failed());
        }
        let data: Data = serde_json::from_slice(&bytes).map_err(|_| storage_failed())?;
        if data.valid() {
            Ok(data)
        } else {
            Err(storage_failed())
        }
    }

    /// The first file, from the MOG configs. It never changes a config and never picks a home
    /// when more than one profile exists.
    fn migrate(&self) -> CoreResult<Data> {
        let list = self.configs.list().map_err(|_| storage_failed())?;
        let mut groups: BTreeMap<String, Vec<MogId>> = BTreeMap::new();
        for item in &list.items {
            let cfg = self.configs.read(&item.id).map_err(|_| storage_failed())?;
            groups
                .entry(cfg.binding.home)
                .or_default()
                .push(item.id.clone());
        }
        let lamp_mog_id = list.items.iter().map(|i| i.id.clone()).min();
        // FLAG 5: an unreadable config may hide another home, so no home is picked then.
        let home = (groups.len() == 1 && list.unreadable.is_empty())
            .then(|| groups.keys().next().cloned())
            .flatten();
        let workspaces = groups
            .into_iter()
            .map(|(profile, mut mog_ids)| {
                mog_ids.sort();
                Entry {
                    profile,
                    mog_ids,
                    available: true,
                }
            })
            .collect();
        Ok(Data {
            schema_version: SCHEMA_VERSION,
            lamp_mog_id,
            home,
            workspaces,
            agent: AgentData {
                genie_call_budget: GenieCallBudget::DEFAULT,
                endpoint: default_model(),
                clean_dictation: true,
                cloud_dictation: false,
                _legacy_run_profile: None,
                _legacy_cross_workspace_data: None,
            },
        })
    }

    fn save(&self, data: &Data) -> CoreResult<()> {
        let bytes = serde_json::to_vec_pretty(data).map_err(|_| storage_failed())?;
        // The cache goes first: a failed write may leave the file in an unknown state.
        *self.cache.lock().unwrap_or_else(|p| p.into_inner()) = None;
        atomic_write(self.dir(), &self.path, &bytes)
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The config of `mog_id` must exist and be bound to `profile`.
    fn check_mog(&self, profile: &str, mog_id: &str) -> CoreResult<()> {
        match self.configs.read(mog_id) {
            Ok(cfg) if cfg.binding.home == profile => Ok(()),
            _ => Err(refused("invalid_args")),
        }
    }

    /// A supported T7 profile, or `profile_invalid`.
    fn supported(&self, profile: &str) -> CoreResult<()> {
        let list = self
            .profiles
            .profiles(false)
            .map_err(|_| refused("host_failed"))?;
        if list.iter().any(|p| p.name == profile && p.supported) {
            Ok(())
        } else {
            Err(refused("profile_invalid"))
        }
    }

    /// The home profile, read only. On the first load the store takes the home from the MOG
    /// configs (the normal migration, `load`); this call changes nothing else. `None` when there
    /// is none or the file cannot be read. It never picks a home when the choice is ambiguous
    /// (ADR 0117, FLAG 5).
    pub fn home(&self) -> Option<ProfileName> {
        let _g = self.guard();
        self.load().ok()?.home
    }

    /// The orchestrator model of the settings (ADR 0122), read only. `None` when the file cannot
    /// be read.
    pub fn orchestrator_model(&self) -> Option<String> {
        let _g = self.guard();
        Some(self.load().ok()?.agent.endpoint.as_str().to_owned())
    }

    /// "Dictation engine: Databricks" (ADR 0123), read only. `false` when the file cannot be
    /// read: dictation then stays on the Mac.
    pub fn cloud_dictation(&self) -> bool {
        let _g = self.guard();
        self.load().is_ok_and(|d| d.agent.cloud_dictation)
    }

    /// As [`Self::cloud_dictation`], but one atomic load: no guard, no cache lock, no disk
    /// (ADR 0123). Used at key-down, where a key-down must not wait on a settings write. The
    /// flag is set by [`Self::load`] (so it starts correct on the first real read) and by
    /// [`Self::settings_write`] (so a write is visible at once, with no extra load).
    pub fn cloud_dictation_fast(&self) -> bool {
        self.cloud_dictation_flag.load(Ordering::SeqCst)
    }

    /// The current T7 read, for a render. A transient failure (or an empty read: no `HOME`, no
    /// config file, mid-rewrite) must never look like proof that every profile is gone, so this
    /// is read before the store guard and is never allowed to drive a write of the file.
    fn t7(&self, refresh: bool) -> CoreResult<Vec<Profile>> {
        self.profiles
            .profiles(refresh)
            .map_err(|_| refused("host_failed"))
    }

    /// `workspace_list`.
    pub fn list(&self, refresh: bool) -> CoreResult<WorkspaceList> {
        let t7 = self.t7(refresh)?;
        let data = {
            let _g = self.guard();
            self.load()?
        };
        self.render(&data, &t7, refresh)
    }

    /// The rows and settings `data` and the current T7 read `t7` give. A stored entry whose
    /// profile is not in `t7` (owner review 2026-11-24: `databricks auth logout --delete` left a
    /// dead row behind) has no row this time; a vanished home renders as `home: None`, so the
    /// hub falls back to its normal "Choose a home workspace" flow instead of a dead home row.
    /// This never writes anything: `data.workspaces`, `data.home` and `data.lamp_mog_id` on disk
    /// stay exactly as they were, `mog_ids` included, so the workspace returns with its old
    /// links the moment its profile answers in `t7` again.
    fn render(&self, data: &Data, t7: &[Profile], refresh: bool) -> CoreResult<WorkspaceList> {
        let mut rows: Vec<WorkspaceRow> = t7
            .iter()
            .map(|p| {
                let entry = data.entry(&p.name);
                let is_home = data.home.as_deref() == Some(p.name.as_str());
                let sign_in = if !p.supported {
                    WorkspaceSignIn::Unsupported
                } else {
                    self.cached_state(&p.name)
                };
                WorkspaceRow {
                    profile: p.name.clone(),
                    host: p.host.clone(),
                    mog_ids: entry.map(|e| e.mog_ids.clone()).unwrap_or_default(),
                    available: is_home || entry.is_some_and(|e| e.available),
                    is_home,
                    sign_in,
                }
            })
            .collect();
        if refresh {
            self.probe(&mut rows);
        }
        rows.sort_by(|a, b| b.is_home.cmp(&a.is_home).then(a.profile.cmp(&b.profile)));
        let home = data
            .home
            .clone()
            .filter(|h| t7.iter().any(|p| &p.name == h));
        Ok(WorkspaceList {
            home,
            lamp_mog_id: data.lamp_mog_id.clone(),
            workspaces: rows,
            agent: AgentSettings {
                genie_call_budget: data.agent.genie_call_budget,
                endpoint: data.agent.endpoint.clone(),
                clean_dictation: data.agent.clean_dictation,
                cloud_dictation: data.agent.cloud_dictation,
            },
        })
    }

    fn cached_state(&self, profile: &str) -> WorkspaceSignIn {
        match self.sign_in.get().map(|s| s.state(profile)) {
            Some(TokenState::Valid | TokenState::Expiring) => WorkspaceSignIn::SignedIn,
            Some(TokenState::LoginRequired) => WorkspaceSignIn::Expired,
            Some(TokenState::Unknown) | None => WorkspaceSignIn::Unknown,
        }
    }

    /// Probe each supported row of unknown state: at most [`PROBE_PARALLEL`] at once, each for
    /// at most the probe timeout. An auth failure is `Expired`; any other failure or a timeout
    /// leaves the row `Unknown`.
    fn probe(&self, rows: &mut [WorkspaceRow]) {
        let Some(source) = self.sign_in.get() else {
            return;
        };
        let todo: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.sign_in == WorkspaceSignIn::Unknown)
            .map(|(i, _)| i)
            .collect();
        // One deadline for the whole call. A slot goes back only when its probe thread really
        // returns, even late, so no more than PROBE_PARALLEL probes ever run at once.
        let deadline = Instant::now() + self.probe_timeout;
        let (tx, rx) = mpsc::channel();
        let mut started = 0;
        for &i in &todo {
            if !self.slots.acquire(deadline) {
                break;
            }
            started += 1;
            let (tx, source, slots) = (tx.clone(), source.clone(), self.slots.clone());
            let (profile, host) = (rows[i].profile.clone(), rows[i].host.clone());
            std::thread::spawn(move || {
                let _slot = SlotGuard(slots);
                let _ = tx.send((i, source.probe(&profile, &host)));
            });
        }
        drop(tx);
        for _ in 0..started {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok((i, Ok(()))) => rows[i].sign_in = WorkspaceSignIn::SignedIn,
                Ok((i, Err(e))) if is_auth_error(&e) => {
                    rows[i].sign_in = WorkspaceSignIn::Expired;
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    /// `workspace_set_available`.
    pub fn set_available(
        &self,
        profile: &str,
        available: bool,
        mog_id: Option<&str>,
    ) -> CoreResult<WorkspaceList> {
        let t7 = self.t7(false)?;
        let data = {
            let _g = self.guard();
            let mut data = self.load()?;
            if data.home.as_deref() == Some(profile) {
                if available {
                    // Home is on already.
                    return self.render(&data, &t7, false);
                }
                return Err(refused("home_always_available"));
            }
            if available {
                self.supported(profile)?;
                let needs_mog = data.entry(profile).is_none_or(|e| e.mog_ids.is_empty());
                if needs_mog {
                    let id = mog_id.ok_or(refused("invalid_args"))?;
                    self.check_mog(profile, id)?;
                    data.entry_mut(profile).mog_ids = vec![id.to_owned()];
                }
            }
            if available || data.entry(profile).is_some() {
                data.entry_mut(profile).available = available;
            }
            self.save(&data)?;
            data
        };
        self.render(&data, &t7, false)
    }

    /// `workspace_make_home`.
    pub fn make_home(&self, profile: &str, mog_id: Option<&str>) -> CoreResult<WorkspaceList> {
        let t7 = self.t7(false)?;
        let data = {
            let _g = self.guard();
            let mut data = self.load()?;
            self.supported(profile)?;
            if let Some(id) = mog_id {
                self.check_mog(profile, id)?;
                let e = data.entry_mut(profile);
                if !e.mog_ids.iter().any(|m| m == id) {
                    e.mog_ids.push(id.to_owned());
                    e.mog_ids.sort();
                }
            }
            data.entry_mut(profile).available = true;
            data.home = Some(profile.to_owned());
            if data.lamp_mog_id.is_none() {
                data.lamp_mog_id = data.entry(profile).and_then(|e| e.mog_ids.first().cloned());
            }
            self.save(&data)?;
            data
        };
        self.render(&data, &t7, false)
    }

    /// The agent session snapshot: the catalog with model aliases, D2, the home profile and
    /// the orchestrator model. It probes nothing. The catalog holds home and each available
    /// profile that T7 supports, home first. A model alias is the profile name in lower case with
    /// every other character turned into `-`, and a repeat gets `-2`, `-3`. The model never sees
    /// the profile name itself in the prompt, only the alias and a display name.
    pub fn catalog_snapshot(&self) -> CoreResult<crate::agent_run::orchestrator::Snapshot> {
        let t7 = self.t7(false)?;
        let data = {
            let _g = self.guard();
            self.load()?
        };
        // The model runs on the AI Gateway of the home profile (ADR 0122), so home is the agent
        // workspace.
        let home = data.home.clone();
        let mut names: Vec<&str> = t7
            .iter()
            .filter(|p| p.supported && data.allows(&p.name))
            .map(|p| p.name.as_str())
            .collect();
        names.sort_by(|a, b| {
            let (ha, hb) = (
                data.home.as_deref() == Some(*a),
                data.home.as_deref() == Some(*b),
            );
            hb.cmp(&ha).then(a.cmp(b))
        });
        let mut used: Vec<String> = Vec::new();
        let mut catalog = Vec::new();
        for name in names {
            let base = alias_slug(name);
            let mut alias = base.clone();
            let mut n = 2;
            while used.contains(&alias) {
                // The whole alias, suffix included, stays within the cap.
                let suffix = format!("-{n}");
                let keep = ALIAS_MAX_CHARS - suffix.len();
                let head = base[..base.len().min(keep)].trim_end_matches('-');
                alias = format!("{head}{suffix}");
                n += 1;
            }
            used.push(alias.clone());
            catalog.push(crate::agent_run::seam::CatalogEntry {
                alias,
                profile: name.to_owned(),
                name: name.to_owned(),
                is_home: data.home.as_deref() == Some(name),
                is_agent_workspace: home.as_deref() == Some(name),
            });
        }
        Ok(crate::agent_run::orchestrator::Snapshot {
            catalog,
            budget: data.agent.genie_call_budget.get(),
            home_profile: home,
            model: data.agent.endpoint.as_str().to_owned(),
        })
    }

    /// `workspace_settings_write`: D2 and the agent endpoint. The argument types hold only
    /// valid values, so a bad value never gets here.
    pub fn settings_write(&self, a: &WorkspaceSettingsWriteArgs) -> CoreResult<WorkspaceList> {
        let t7 = self.t7(false)?;
        let data = {
            let _g = self.guard();
            let mut data = self.load()?;
            if let Some(v) = a.genie_call_budget {
                data.agent.genie_call_budget = v;
            }
            if let Some(v) = &a.agent_endpoint {
                data.agent.endpoint = v.clone();
            }
            if let Some(v) = a.clean_dictation {
                data.agent.clean_dictation = v;
            }
            if let Some(v) = a.cloud_dictation {
                data.agent.cloud_dictation = v;
            }
            self.save(&data)?;
            // Only after a successful save: a failed write must not make the fast flag
            // (ADR 0123) disagree with the file that is actually on disk.
            if let Some(v) = a.cloud_dictation {
                self.cloud_dictation_flag.store(v, Ordering::SeqCst);
            }
            data
        };
        self.render(&data, &t7, false)
    }
}

impl WorkspaceGate for WorkspaceStore {
    fn allows(&self, profile: &str) -> bool {
        let _g = self.guard();
        self.load().is_ok_and(|d| d.allows(profile))
    }
    fn workspace_url(&self, profile: &str) -> Option<String> {
        let list = self.profiles.profiles(false).ok()?;
        list.into_iter()
            .find(|p| p.name == profile && p.supported)
            .map(|p| p.host)
    }
}

fn args_of<T: for<'de> Deserialize<'de>>(args: Value) -> CoreResult<T> {
    serde_json::from_value(args).map_err(|_| refused("invalid_args"))
}

impl GatewayComponents for WorkspaceStore {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        let list = match command {
            "workspace_list" => {
                let a: WorkspaceListArgs = args_of(args)?;
                self.list(a.refresh.unwrap_or(false))?
            }
            "workspace_set_available" => {
                let a: WorkspaceSetAvailableArgs = args_of(args)?;
                self.set_available(&a.profile, a.available, a.mog_id.as_deref())?
            }
            "workspace_make_home" => {
                let a: WorkspaceMakeHomeArgs = args_of(args)?;
                self.make_home(&a.profile, a.mog_id.as_deref())?
            }
            "workspace_settings_write" => self.settings_write(&args_of(args)?)?,
            _ => return Err(refused("not_implemented")),
        };
        serde_json::to_value(list).map_err(|_| refused("host_failed"))
    }
}

#[cfg(test)]
mod tests;
