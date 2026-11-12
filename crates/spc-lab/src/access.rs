//! Expanded-access claims, authentication, scope and user projection.
//!
//! This is a first foundation for identity-scoped lab access (live-lab plan
//! section 5), not a production auth platform. It adds no generic RPC proxy,
//! no per-request token rotation and no second controller.
//!
//! Opt-in only: a scenario without `expanded_access: true` never touches this
//! module's behavior - the unauthenticated loopback surface is unchanged. In
//! the expanded profile the runtime generates one 0600 claim file per SPc user
//! plus one operator claim under `<run>/private/access/` (directory 0700),
//! every controller route except the closed public static bootstrap list
//! requires a Bearer token, the effective actor/run/user/pool/miner/worker
//! ids are bound server-side from the stored claim before any effect, and
//! reads are served through a server-side projection that filters before
//! JSON serialization.
//!
//! Fail-closed rules throughout: a missing, unknown, malformed or revoked
//! token rejects before any journal entry, effect or stored-outcome lookup,
//! with no private error echo; an identifier is never used as a raw path
//! component (unsafe components are SHA-256 hashed into a safe filename); a
//! bearer token is never placed in a URL, query, log or shareable artifact.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::command::CommandKind;
use crate::evidence::{EvidenceEvent, EvidencePayload, Snapshot};
use crate::ids::{ActorId, IdentityId, MinerId, PoolId, RunId, WorkerId};
use crate::scenario::Scenario;

/// Schema tag persisted in every claim file and echoed in the `/me` view.
pub const ACCESS_CLAIM_SCHEMA: &str = "spc-access-claim/v1";
/// Schema tag of the filtered user snapshot DTO (`spc-user-snapshot/v1`).
pub const USER_SNAPSHOT_SCHEMA: &str = "spc-user-snapshot/v1";
/// Schema tag of the filtered user SSE event DTO (`spc-user-event/v1`).
pub const USER_EVENT_SCHEMA: &str = "spc-user-event/v1";
/// Schema tag of the miner-poll worker projection (`spc-user-projection/v1`).
pub const USER_PROJECTION_SCHEMA: &str = "spc-user-projection/v1";

/// Principal string for in-process harness submissions (CLI/runtime phases).
/// It is an authenticated principal for journal binding, never a browser
/// authority: the HTTP layer cannot produce it.
pub const PRINCIPAL_CONTROLLER: &str = "controller";
/// Principal string of the operator claim.
pub const PRINCIPAL_OPERATOR: &str = "operator";

/// Token size: 32 bytes of CSPRNG output, hex encoded in the claim file.
pub const TOKEN_BYTES: usize = 32;

/// The workbench Content-Security-Policy served on static and relayed HTML
/// in the expanded profile only.
pub const WORKBENCH_CSP: &str =
    "default-src 'self'; connect-src 'self'; form-action 'self'; base-uri 'none'";

/// The closed relay asset list: each allowlisted asset maps to a fixed MIME
/// type. `status` is the one named status endpoint (Bearer required, operator
/// only in this foundation). Unknown assets are 404; there is no wildcard.
pub const RELAY_STATIC_ASSETS: &[(&str, &str)] = &[
    ("index.html", "text/html; charset=utf-8"),
    ("workbench.css", "text/css; charset=utf-8"),
    ("workbench.js", "text/javascript; charset=utf-8"),
];
pub const RELAY_STATUS_ASSET: &str = "status";
/// Hard cap on a relayed upstream body. `namespace::http_request` already
/// bounds the whole response at 8 MiB / 30 s; the relay keeps its own tighter
/// body bound so a closed asset can never become a bulk pipe.
pub const RELAY_MAX_BODY_BYTES: usize = 1024 * 1024;

/// Resolve a relay asset to its fixed content type, or `None` when the asset
/// is not in the closed list.
pub fn relay_asset_content_type(asset: &str) -> Option<&'static str> {
    if asset == RELAY_STATUS_ASSET {
        return Some("application/json");
    }
    RELAY_STATIC_ASSETS
        .iter()
        .find(|(name, _)| *name == asset)
        .map(|(_, content_type)| *content_type)
}

/// Candidate command kinds that exist as typed T0 traffic payloads but have
/// no runtime variant yet. In the expanded profile they are detected
/// pre-parse and answered with a closed `not available` outcome - never
/// journaled, never pretended to be T1.
pub const UNAVAILABLE_KINDS: &[&str] = &["send_user_payment", "bootstrap_fund"];

pub fn is_unavailable_kind(kind_tag: &str) -> bool {
    UNAVAILABLE_KINDS.contains(&kind_tag)
}

// ---------------------------------------------------------------------------
// Claim derivation from the scenario
// ---------------------------------------------------------------------------

/// The per-user bindings a claim asserts. Derived from the scenario, never
/// from request input.
#[derive(Debug, Clone)]
pub struct UserBinding {
    pub user_id: String,
    pub pool_id: PoolId,
    pub identity_id: IdentityId,
    pub miner_id: MinerId,
    pub worker_ids: Vec<WorkerId>,
}

/// Derive one binding per identity. Validation is load-bearing for scope
/// safety (the claim's worker list is the authority for user commands and
/// reads; the prepare-time shape below must also hold):
///
/// - every identity is referenced by exactly one miner's connections;
/// - that miner's connections all reference that identity (no shared miner);
/// - the identity has at least one declared worker;
/// - the miner's connections resolve to exactly one pool through their
///   fronts;
/// - the identity id is not a reserved principal (`operator`,
///   `controller`) and does not start with `sha256-` (the hashed-filename
///   namespace), and no two bindings collide on one claim filename.
///
/// Any ambiguity is a closed error before any filesystem, lock or authority
/// effect - the access directory is not created.
pub fn derive_user_bindings(scenario: &Scenario) -> Result<Vec<UserBinding>, String> {
    let mut bindings = Vec::new();
    let mut filenames = vec![claim_filename(PRINCIPAL_OPERATOR)];
    for identity in &scenario.identities {
        let user_id = identity.id.as_str();
        if user_id == PRINCIPAL_OPERATOR || user_id == PRINCIPAL_CONTROLLER {
            return Err(format!(
                "expanded_access reserves the principal {user_id:?}; an identity id may not use it"
            ));
        }
        if user_id.starts_with("sha256-") {
            return Err(format!(
                "expanded_access reserves the sha256- filename prefix; identity id {user_id:?} \
                 may not use it"
            ));
        }
        let filename = claim_filename(user_id);
        if filenames.contains(&filename) {
            return Err(format!(
                "expanded_access claim filename collision on {filename:?} for identity {user_id}"
            ));
        }
        filenames.push(filename);
    }
    for identity in &scenario.identities {
        let mut owning_miners: Vec<&crate::scenario::MinerSpec> = Vec::new();
        for miner in &scenario.miners {
            if miner
                .connections
                .iter()
                .any(|connection| connection.identity_id == identity.id)
            {
                owning_miners.push(miner);
            }
        }
        if owning_miners.len() != 1 {
            return Err(format!(
                "expanded_access requires identity {} to be referenced by exactly one miner's \
                 connections (found {})",
                identity.id,
                owning_miners.len()
            ));
        }
        let miner = owning_miners[0];
        if miner.connections.is_empty() {
            return Err(format!(
                "expanded_access requires miner {} to declare at least one connection",
                miner.id
            ));
        }
        for connection in &miner.connections {
            if connection.identity_id != identity.id {
                return Err(format!(
                    "expanded_access requires miner {} to serve exactly one identity (connection \
                     {} references {})",
                    miner.id, connection.id, connection.identity_id
                ));
            }
        }
        let mut pool_ids: Vec<&PoolId> = Vec::new();
        for connection in &miner.connections {
            let front = scenario
                .fronts
                .iter()
                .find(|front| front.id == connection.front_id)
                .ok_or_else(|| {
                    format!(
                        "expanded_access: connection {} references unknown front {}",
                        connection.id, connection.front_id
                    )
                })?;
            if !pool_ids.contains(&&front.pool_id) {
                pool_ids.push(&front.pool_id);
            }
        }
        if pool_ids.len() != 1 {
            return Err(format!(
                "expanded_access requires miner {} connections to resolve to exactly one pool \
                 (found {})",
                miner.id,
                pool_ids.len()
            ));
        }
        let worker_ids: Vec<WorkerId> = miner
            .workers
            .iter()
            .filter(|worker| worker.identity_id == identity.id)
            .map(|worker| worker.id.clone())
            .collect();
        if worker_ids.is_empty() {
            return Err(format!(
                "expanded_access requires identity {} to have at least one declared worker",
                identity.id
            ));
        }
        bindings.push(UserBinding {
            user_id: identity.id.as_str().to_string(),
            pool_id: (*pool_ids[0]).clone(),
            identity_id: identity.id.clone(),
            miner_id: miner.id.clone(),
            worker_ids,
        });
    }
    Ok(bindings)
}

// ---------------------------------------------------------------------------
// Claim record and store
// ---------------------------------------------------------------------------

/// One access claim. The user fields form the plan's contract shape
/// (`token`, `user_id`, `pool_id`, `identity_id`, `miner_id`, `worker_ids`,
/// `actor`, `may`); `schema`, `run_id` and `principal` are an additive
/// superset. The operator claim carries `may: ["operator"]` and leaves the
/// user fields absent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessClaim {
    pub schema: String,
    pub run_id: RunId,
    pub principal: String,
    /// One lab-local secret for this run only. Never in a URL, query, log,
    /// shareable artifact or the hub feed.
    pub token: String,
    /// The `LabCommand.actor` this claim binds server-side.
    pub actor: ActorId,
    pub may: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_id: Option<PoolId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_id: Option<IdentityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub miner_id: Option<MinerId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_ids: Option<Vec<WorkerId>>,
}

impl AccessClaim {
    pub fn is_operator(&self) -> bool {
        self.may.iter().any(|entry| entry == "operator")
    }

    pub fn is_user(&self) -> bool {
        !self.is_operator()
    }
}

/// The authenticated context produced by a successful token check.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub principal: String,
    pub claim: AccessClaim,
}

struct AccessState {
    /// principal -> claim.
    claims: BTreeMap<String, AccessClaim>,
}

/// The run's claim store: one directory of 0600 JSON claim files.
///
/// Every authentication re-validates and reloads the whole directory (a
/// handful of small secret files at lab scale - no cached authority).
/// Revocation is therefore claim-file deletion or an in-place rewrite
/// (token rotation, scope reduction), and both take effect on the very
/// next request with no directory-change signal to wait for. A directory
/// that fails to load - unreadable, badly permissioned, unparseable,
/// inconsistent - denies every token, the operator included, until it is
/// repaired: fail closed all-or-nothing, never forgiven per file.
pub struct AccessStore {
    dir: PathBuf,
    state: Mutex<AccessState>,
}

impl AccessStore {
    /// Generate the operator claim and one claim per user binding under
    /// `<run_dir>/private/access/` (0700 directory, 0600 atomic files).
    pub fn prepare(run_id: &RunId, run_dir: &Path, scenario: &Scenario) -> Result<Self, String> {
        let bindings = derive_user_bindings(scenario)?;
        let dir = run_dir.join("private").join("access");
        Self::create_dir(&dir)?;
        let mut claims = BTreeMap::new();
        let operator = AccessClaim {
            schema: ACCESS_CLAIM_SCHEMA.to_string(),
            run_id: run_id.clone(),
            principal: PRINCIPAL_OPERATOR.to_string(),
            token: generate_token(),
            actor: ActorId::trusted(PRINCIPAL_OPERATOR),
            may: vec!["operator".to_string()],
            user_id: None,
            pool_id: None,
            identity_id: None,
            miner_id: None,
            worker_ids: None,
        };
        Self::write_claim(&dir, &operator)?;
        claims.insert(operator.principal.clone(), operator);
        for binding in bindings {
            let claim = AccessClaim {
                schema: ACCESS_CLAIM_SCHEMA.to_string(),
                run_id: run_id.clone(),
                principal: binding.user_id.clone(),
                token: generate_token(),
                actor: ActorId::trusted(format!("user/{}", binding.user_id)),
                may: vec![
                    "pause_resume_workers".to_string(),
                    "set_worker_budget".to_string(),
                    "send_user_payment".to_string(),
                    "view_own_status".to_string(),
                    "wallet_refresh".to_string(),
                    "wallet_offboard".to_string(),
                ],
                user_id: Some(binding.user_id.clone()),
                pool_id: Some(binding.pool_id),
                identity_id: Some(binding.identity_id),
                miner_id: Some(binding.miner_id),
                worker_ids: Some(binding.worker_ids),
            };
            Self::write_claim(&dir, &claim)?;
            claims.insert(claim.principal.clone(), claim);
        }
        Ok(Self {
            dir,
            state: Mutex::new(AccessState { claims }),
        })
    }

    /// Reopen an existing claim directory for an expanded run. The operator
    /// claim must be present; a missing store is a closed error, never a
    /// fallback to unauthenticated loopback.
    pub fn attach(dir: impl AsRef<Path>) -> Result<Self, String> {
        let dir = dir.as_ref().to_path_buf();
        let claims = Self::load_dir(&dir)?;
        if !claims.values().any(AccessClaim::is_operator) {
            return Err(format!(
                "expanded access requires the operator claim in {}",
                dir.display()
            ));
        }
        Ok(Self {
            dir,
            state: Mutex::new(AccessState { claims }),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// All claims (operator and users), ordered by principal.
    pub fn claims(&self) -> Vec<AccessClaim> {
        self.state
            .lock()
            .unwrap()
            .claims
            .values()
            .cloned()
            .collect()
    }

    /// Authenticate a bearer token. Constant-time comparison on the decoded
    /// 32-byte tokens; missing/unknown/revoked/malformed all return `None`
    /// with no distinction visible to the caller.
    pub fn authenticate(&self, token_hex: &str) -> Option<AuthContext> {
        let token = decode_token(token_hex)?;
        let mut state = self.state.lock().unwrap();
        // Unconditional validated reload: a deleted, rewritten (rotated or
        // re-scoped) or corrupted claim file is observed on the very next
        // request, with no directory-change signal to wait for. A failed
        // load clears the whole store: every token is denied until the
        // directory is repaired - all-or-nothing, never forgiven per file.
        match Self::load_dir(&self.dir) {
            Ok(claims) => state.claims = claims,
            Err(_) => state.claims.clear(),
        }
        let (principal, claim) = state
            .claims
            .iter()
            .find(|(_, claim)| {
                decode_token(&claim.token).is_some_and(|stored| constant_time_eq(&token, &stored))
            })
            .map(|(principal, claim)| (principal.clone(), claim.clone()))?;
        Some(AuthContext { principal, claim })
    }

    /// Revoke one principal by deleting its claim file. The next
    /// authentication attempt re-reads the directory and fails closed.
    pub fn revoke(&self, principal: &str) -> bool {
        let path = self.dir.join(claim_filename(principal));
        fs::remove_file(&path).is_ok()
    }

    fn create_dir(dir: &Path) -> Result<(), String> {
        fs::create_dir_all(dir)
            .and_then(|_| fs::set_permissions(dir, fs::Permissions::from_mode(0o700)))
            .map_err(|e| format!("access claim directory {}: {e}", dir.display()))
    }

    /// Load and fully validate the claim directory. Every check is a closed
    /// error (which `authenticate` turns into deny-all):
    ///
    /// - the directory itself must be a real directory (no symlink - the
    ///   path is lstat'ed, never followed), mode `0700` (no group/world
    ///   access), owned by this process;
    /// - every `*.json` entry must be a regular file (no symlink, fifo or
    ///   directory), mode `0600`, owned by this process;
    /// - every file must parse as a claim with the known schema;
    /// - the principal namespace must be consistent: no file may claim the
    ///   reserved `controller` principal, exactly the `operator` principal
    ///   may carry operator authority, and no other principal may;
    /// - no two files may claim the same principal or the same token -
    ///   duplicate authority is a closed error, never last-wins.
    fn load_dir(dir: &Path) -> Result<BTreeMap<String, AccessClaim>, String> {
        let dir_meta = fs::symlink_metadata(dir)
            .map_err(|e| format!("access claim directory {}: {e}", dir.display()))?;
        if !dir_meta.is_dir() {
            return Err(format!(
                "access claim directory {} is not a directory",
                dir.display()
            ));
        }
        if dir_meta.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "access claim directory {} is group or world accessible",
                dir.display()
            ));
        }
        if dir_meta.uid() != current_uid() {
            return Err(format!(
                "access claim directory {} is not owned by this process",
                dir.display()
            ));
        }
        let entries = fs::read_dir(dir)
            .map_err(|e| format!("access claim directory {}: {e}", dir.display()))?;
        let mut claims = BTreeMap::new();
        let mut tokens = std::collections::BTreeSet::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let meta = fs::symlink_metadata(&path)
                .map_err(|e| format!("access claim {}: {e}", path.display()))?;
            if !meta.is_file() {
                return Err(format!(
                    "access claim {} is not a regular file",
                    path.display()
                ));
            }
            if meta.permissions().mode() & 0o077 != 0 {
                return Err(format!(
                    "access claim {} is group or world readable",
                    path.display()
                ));
            }
            if meta.uid() != current_uid() {
                return Err(format!(
                    "access claim {} is not owned by this process",
                    path.display()
                ));
            }
            let text = fs::read_to_string(&path)
                .map_err(|e| format!("access claim {}: {e}", path.display()))?;
            let claim: AccessClaim = serde_json::from_str(&text).map_err(|e| {
                format!("access claim {} is not a valid claim: {e}", path.display())
            })?;
            if claim.schema != ACCESS_CLAIM_SCHEMA {
                return Err(format!(
                    "access claim {} has unknown schema {:?}",
                    path.display(),
                    claim.schema
                ));
            }
            if claim.principal == PRINCIPAL_CONTROLLER {
                return Err(format!(
                    "access claim {} claims the reserved controller principal",
                    path.display()
                ));
            }
            if claim.is_operator() != (claim.principal == PRINCIPAL_OPERATOR) {
                return Err(format!(
                    "access claim {} mixes principal and authority namespace",
                    path.display()
                ));
            }
            if claims.contains_key(&claim.principal) {
                return Err(format!(
                    "access claim {} duplicates the principal of another claim file",
                    path.display()
                ));
            }
            if !tokens.insert(claim.token.clone()) {
                return Err(format!(
                    "access claim {} duplicates another claim file's token",
                    path.display()
                ));
            }
            claims.insert(claim.principal.clone(), claim);
        }
        Ok(claims)
    }

    /// Write one claim file atomically: a 0600 temp file in the same
    /// directory, synced, then renamed over the target.
    fn write_claim(dir: &Path, claim: &AccessClaim) -> Result<(), String> {
        let body = serde_json::to_vec_pretty(claim)
            .map_err(|e| format!("claim encode for {}: {e}", claim.principal))?;
        let target = dir.join(claim_filename(&claim.principal));
        let temp = dir.join(format!(
            ".tmp-claim-{}",
            hex::encode(Sha256::digest(
                format!("{}-{}", claim.principal, claim.token).as_bytes()
            ))
        ));
        write_private_atomic(&temp, &target, &body)
    }
}

/// Claim file name for a principal. A principal id may legitimately contain
/// `/`, `.` or `..` (identifier validation allows them), so an id is used as
/// a filename only when it is a single safe component; anything else is
/// SHA-256 hashed into a safe name. A raw path join never happens.
fn claim_filename(principal: &str) -> String {
    match safe_filename_component(principal) {
        Some(name) => name,
        None => format!(
            "sha256-{}.json",
            hex::encode(Sha256::digest(principal.as_bytes()))
        ),
    }
}

fn safe_filename_component(principal: &str) -> Option<String> {
    if principal.is_empty() || principal.len() > 128 {
        return None;
    }
    if principal == "." || principal == ".." {
        return None;
    }
    let safe = principal
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':' | b'@'));
    if !safe {
        return None;
    }
    Some(format!("{principal}.json"))
}

fn write_private_atomic(temp: &Path, target: &Path, body: &[u8]) -> Result<(), String> {
    use std::io::Write;
    {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(temp)
            .map_err(|e| format!("claim temp file {}: {e}", temp.display()))?;
        file.write_all(body)
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("claim temp file {}: {e}", temp.display()))?;
    }
    fs::rename(temp, target).map_err(|e| {
        let _ = fs::remove_file(temp);
        format!("claim file {}: {e}", target.display())
    })?;
    let _ = fs::set_permissions(target, fs::Permissions::from_mode(0o600));
    Ok(())
}

fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// The uid of this process, used to reject claim files and directories
/// owned by someone else (`/proc/self` is owned by the process owner; the
/// crate targets Linux).
fn current_uid() -> u32 {
    fs::metadata("/proc/self")
        .map(|meta| meta.uid())
        .unwrap_or(0)
}

/// Decode a hex-encoded 32-byte token. Anything else is malformed.
fn decode_token(value: &str) -> Option<[u8; TOKEN_BYTES]> {
    let bytes = hex::decode(value.trim()).ok()?;
    if bytes.len() != TOKEN_BYTES {
        return None;
    }
    let mut token = [0u8; TOKEN_BYTES];
    token.copy_from_slice(&bytes);
    Some(token)
}

/// Constant-time equality over fixed-size tokens.
fn constant_time_eq(a: &[u8; TOKEN_BYTES], b: &[u8; TOKEN_BYTES]) -> bool {
    let mut diff = 0u8;
    for (left, right) in a.iter().zip(b.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

// ---------------------------------------------------------------------------
// Command scope
// ---------------------------------------------------------------------------

/// Check a command against a claim before any journal lookup or effect.
///
/// The claim, not the payload, is the authority: the payload `actor` must
/// equal the claim's bound actor (never silently overwritten), and every id
/// inside `kind` is validated against the claim's bound miner/workers. A
/// user may issue only `PauseWorkers`/`ResumeWorkers`/`SetWorkerBudget` on
/// its own miner, naming an explicit non-empty subset of its own workers
/// (the browser knows its ids from `/me`); everything else requires the
/// operator claim.
pub fn check_command_scope(
    claim: &AccessClaim,
    command: &crate::command::LabCommand,
) -> Result<(), String> {
    if command.actor.as_str() != claim.actor.as_str() {
        return Err("command actor does not match the authenticated claim".to_string());
    }
    if crate::wallet_control::identity(&command.kind).is_some() {
        crate::wallet_control::validate(&command.kind).map_err(str::to_string)?;
    }
    if claim.is_operator() {
        return Ok(());
    }
    let worker_ids = claim.worker_ids.as_deref().unwrap_or(&[]);
    match &command.kind {
        CommandKind::SendUserPayment {
            user,
            destination_address,
            amount_sats,
            fee_rate_sats_vb,
        } => {
            if claim.identity_id.as_ref() != Some(user) {
                return Err("payment user does not belong to the authenticated claim".into());
            }
            crate::traffic::commands::SendUserPayment {
                user: user.clone(),
                destination_address: destination_address.clone(),
                amount_sats: *amount_sats,
                fee_rate_sats_vb: *fee_rate_sats_vb,
            }
            .validate()
            .map_err(|_| "invalid user payment".to_string())
        }
        CommandKind::WalletRefresh { identity_id, .. }
        | CommandKind::WalletOffboard { identity_id, .. }
        | CommandKind::WalletReadInputs { identity_id } => {
            let capability = match &command.kind {
                CommandKind::WalletRefresh { .. } => "wallet_refresh",
                CommandKind::WalletOffboard { .. } => "wallet_offboard",
                CommandKind::WalletReadInputs { .. } => "view_own_status",
                _ => unreachable!(),
            };
            if claim.identity_id.as_ref() != Some(identity_id)
                || !claim.may.iter().any(|m| m == capability)
            {
                return Err("wallet does not belong to the authenticated claim".to_string());
            }
            crate::wallet_control::validate(&command.kind).map_err(str::to_string)
        }
        CommandKind::PauseWorkers { miner_id, workers }
        | CommandKind::ResumeWorkers { miner_id, workers } => {
            if Some(miner_id) != claim.miner_id.as_ref() {
                return Err("miner does not belong to the authenticated claim".to_string());
            }
            // A user must name a non-empty subset of its own workers. An
            // empty list means "the whole miner" and stays operator-only:
            // the prepare-time one-identity-per-miner binding is a snapshot
            // that operator topology commands can invalidate at runtime, so
            // user scope never leans on it.
            if workers.is_empty() {
                return Err(
                    "worker list must name the claim's own workers (an empty list is \
                     operator-only)"
                        .to_string(),
                );
            }
            if workers.iter().any(|worker| !worker_ids.contains(worker)) {
                return Err("worker does not belong to the authenticated claim".to_string());
            }
            Ok(())
        }
        CommandKind::SetWorkerBudget {
            miner_id,
            worker_id,
            ..
        } => {
            if Some(miner_id) != claim.miner_id.as_ref() {
                return Err("miner does not belong to the authenticated claim".to_string());
            }
            if !worker_ids.contains(worker_id) {
                return Err("worker does not belong to the authenticated claim".to_string());
            }
            Ok(())
        }
        _ => Err("command kind is not permitted for this principal".to_string()),
    }
}

/// The closed outcome for a scope failure or an unavailable kind. It is a
/// real `CommandOutcome` with no journal effect and no private echo.
pub fn closed_rejection(
    command_id: &crate::ids::CommandId,
    detail: &str,
) -> crate::command::CommandOutcome {
    crate::command::CommandOutcome::new(
        command_id.clone(),
        crate::command::OutcomeState::RejectedPrecondition,
        detail,
    )
}

/// The `/me` view of a claim: the contract fields without the token.
pub fn me_view(claim: &AccessClaim) -> serde_json::Value {
    let mut view = serde_json::json!({
        "schema": claim.schema,
        "run_id": claim.run_id,
        "principal": claim.principal,
        "actor": claim.actor,
        "may": claim.may,
    });
    if let Some(user_id) = &claim.user_id {
        view["user_id"] = serde_json::json!(user_id);
    }
    if let Some(pool_id) = &claim.pool_id {
        view["pool_id"] = serde_json::json!(pool_id);
    }
    if let Some(identity_id) = &claim.identity_id {
        view["identity_id"] = serde_json::json!(identity_id);
    }
    if let Some(miner_id) = &claim.miner_id {
        view["miner_id"] = serde_json::json!(miner_id);
    }
    if let Some(worker_ids) = &claim.worker_ids {
        view["worker_ids"] = serde_json::json!(worker_ids);
    }
    view
}

// ---------------------------------------------------------------------------
// Read projection: snapshot, events, worker status
// ---------------------------------------------------------------------------

/// A user-visible evidence event. Only allowlisted kinds with the user's own
/// ids reach this DTO; the raw payload of a miner event is stripped to a
/// typed `{miner_id, name}` projection before serialization.
#[derive(Debug, Clone, Serialize)]
pub struct FilteredEvent {
    pub schema: &'static str,
    pub run_id: RunId,
    pub event_id: u64,
    pub occurred_at_wall: u64,
    pub kind: String,
    pub data: serde_json::Value,
}

/// Filter one evidence event for a user claim, before serialization.
///
/// Visible: `Submitted`/`Accounted`/`Delivered` for the claim's own identity,
/// `MinerEvent` for the claim's own miner (raw payload stripped), and
/// `EventLost` (a gap stays an explicit gap fact). Everything else -
/// including other users' events, job, process, command, topology and
/// unknown kinds - is filtered out entirely.
pub fn filter_event(claim: &AccessClaim, event: &EvidenceEvent) -> Option<FilteredEvent> {
    let identity_id = claim.identity_id.as_ref()?;
    let miner_id = claim.miner_id.as_ref()?;
    let data = match &event.payload {
        EvidencePayload::Submitted(value) if &value.identity_id == identity_id => {
            serde_json::json!({
                "miner_id": value.miner_id,
                "share_id": value.share_id,
                "job_id": value.job_id,
            })
        }
        EvidencePayload::Accounted(value) if &value.identity_id == identity_id => {
            serde_json::json!({
                "share_id": value.share_id,
                "job_id": value.job_id,
                "generation": value.generation,
            })
        }
        EvidencePayload::Delivered(value) if &value.identity_id == identity_id => {
            serde_json::json!({
                "block_hash": value.block_hash,
                "amount_sats": value.amount_sats,
            })
        }
        EvidencePayload::MinerEvent {
            miner_id: own,
            name,
            ..
        } if own == miner_id => {
            // The raw miner JSON payload never enters a user response.
            serde_json::json!({
                "miner_id": own,
                "name": name,
            })
        }
        EvidencePayload::EventLost(gap) => serde_json::json!({
            "scope": gap.scope,
            "from_event_id": gap.from_event_id,
            "to_event_id": gap.to_event_id,
            "reason": gap.reason,
        }),
        _ => return None,
    };
    Some(FilteredEvent {
        schema: USER_EVENT_SCHEMA,
        run_id: event.run_id.clone(),
        event_id: event.event_id,
        occurred_at_wall: event.occurred_at_wall,
        kind: event.payload.kind().to_string(),
        data,
    })
}

/// The filtered user snapshot (`spc-user-snapshot/v1`), built from the
/// whole-run snapshot before serialization: only the user's own miner and
/// worker rows, the global watermark, explicit gaps, and `not_available`
/// wallet/payment facts (never zeros). `filtered: true` distinguishes a
/// filtered view from the operator's whole-run snapshot.
pub fn user_snapshot(claim: &AccessClaim, snapshot: &Snapshot) -> serde_json::Value {
    let own_miner = claim.miner_id.as_ref();
    let own_workers: Vec<&crate::evidence::EntitySnapshot> = snapshot
        .entities
        .iter()
        .filter(|entity| {
            entity.kind == "worker"
                && entity.miner_id.as_ref() == own_miner
                && claim
                    .worker_ids
                    .as_ref()
                    .is_some_and(|ids| ids.iter().any(|id| id.as_str() == entity.id))
        })
        .collect();
    serde_json::json!({
        "schema": USER_SNAPSHOT_SCHEMA,
        "run_id": snapshot.run_id,
        "principal": claim.principal,
        "user_id": claim.user_id,
        "cursor": snapshot.cursor,
        "retained_from": snapshot.retained_from,
        "resync_required": snapshot.resync_required,
        "snapshot_cursor": snapshot.snapshot_cursor,
        "generated_at": snapshot.generated_at,
        "filtered": true,
        "miner": own_miner,
        "workers": own_workers,
        "gaps": snapshot.gaps,
        "evidence": {
            // Global watermark and gap facts only: counts per kind are not
            // included because they aggregate every user's events.
            "last_event_id": snapshot.evidence.last_event_id,
            "gap_count": snapshot.evidence.gap_count,
        },
        "wallet": {
            "state": "not_available",
            "reason": "AP-3b wallet status interface is not wired",
        },
        "payment_wallet": {
            "state": "not_available",
            "reason": "T1b payment interfaces are not wired",
        },
    })
}

/// One worker row of `spc-user-projection/v1`. Unknown fields stay `null`
/// with a reason - never zeros.
#[derive(Debug, Clone, Serialize)]
pub struct WorkerProjection {
    pub worker_id: WorkerId,
    pub requested_hps: Option<u64>,
    pub achieved_hps: Option<u64>,
    pub active_job_id: Option<String>,
    pub observed_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The miner-poll worker projection (`spc-user-projection/v1`). Sourced from
/// the engine's existing single miner v1 control poll - there is no second
/// pipe. Each worker entry carries its own observation age and error.
#[derive(Debug, Clone, Serialize)]
pub struct UserProjection {
    pub payment_wallet: Option<crate::traffic::payment_view::PaymentWalletView>,
    pub schema: &'static str,
    pub user_id: String,
    pub observed_at: u64,
    pub workers: Vec<WorkerProjection>,
}

impl UserProjection {
    fn base(claim: &AccessClaim, observed_at: u64) -> Self {
        Self {
            schema: USER_PROJECTION_SCHEMA,
            payment_wallet: None,
            user_id: claim.user_id.clone().unwrap_or_default(),
            observed_at,
            workers: Vec::new(),
        }
    }

    /// Build from the claim's worker ids and the miner v1 snapshot result
    /// rows. A missing row or a failed poll yields `null` fields plus the
    /// reason, never zeros.
    pub fn from_miner_snapshot(
        claim: &AccessClaim,
        rows: Option<&[serde_json::Value]>,
        error: Option<&str>,
        observed_at: u64,
    ) -> Self {
        let mut projection = Self::base(claim, observed_at);
        let worker_ids = claim.worker_ids.clone().unwrap_or_default();
        for worker_id in worker_ids {
            let row = rows.and_then(|rows| {
                rows.iter().find(|row| {
                    row.get("id").and_then(|value| value.as_str()) == Some(worker_id.as_str())
                })
            });
            let entry = match (error, row) {
                (Some(error), _) => WorkerProjection {
                    worker_id: worker_id.clone(),
                    requested_hps: None,
                    achieved_hps: None,
                    active_job_id: None,
                    observed_at,
                    error: Some(error.to_string()),
                },
                (None, Some(row)) => {
                    let mut entry = WorkerProjection {
                        worker_id: worker_id.clone(),
                        requested_hps: row
                            .pointer("/requested/hashes_per_second")
                            .and_then(|value| value.as_u64()),
                        achieved_hps: row.get("achieved_hps").and_then(|value| value.as_u64()),
                        active_job_id: row
                            .get("active_job_id")
                            .and_then(|value| value.as_str())
                            .map(str::to_string),
                        observed_at,
                        error: None,
                    };
                    if entry.requested_hps.is_none()
                        && entry.achieved_hps.is_none()
                        && entry.active_job_id.is_none()
                    {
                        entry.error = Some("miner snapshot fields are absent".to_string());
                    }
                    entry
                }
                (None, None) => WorkerProjection {
                    worker_id: worker_id.clone(),
                    requested_hps: None,
                    achieved_hps: None,
                    active_job_id: None,
                    observed_at,
                    error: Some("worker row is missing from the miner snapshot".to_string()),
                },
            };
            projection.workers.push(entry);
        }
        projection
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_principal_ids_never_become_raw_paths() {
        for principal in ["u0/../etc", "../escape", "a/b", "..", "."] {
            let name = claim_filename(principal);
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'),
                "{principal} -> {name}"
            );
            assert!(!name.contains('/'), "{principal} -> {name}");
            assert!(!name.starts_with('.'), "{principal} -> {name}");
            assert!(name.ends_with(".json"), "{principal} -> {name}");
        }
        assert_eq!(claim_filename("id-alice"), "id-alice.json");
    }

    #[test]
    fn token_compare_and_decode_are_strict() {
        let token = generate_token();
        assert_eq!(token.len(), TOKEN_BYTES * 2);
        let decoded = decode_token(&token).unwrap();
        assert!(constant_time_eq(&decoded, &decoded));
        assert!(decode_token("nothex").is_none());
        assert!(decode_token(&token[..TOKEN_BYTES]).is_none());
        assert!(decode_token("").is_none());
    }

    #[test]
    fn generated_tokens_are_csprng_distinct() {
        // 64 draws, 64 distinct values: no degenerate RNG state and no
        // constant token. (Distinctness is observable; entropy quality is
        // the OS RNG's contract.)
        let tokens: std::collections::BTreeSet<String> =
            (0..64).map(|_| generate_token()).collect();
        assert_eq!(tokens.len(), 64);
    }

    #[test]
    fn user_snapshot_whitelist_is_the_claim_worker_list() {
        // A worker entity that merely matches the claim's miner id but is
        // not in the claim's worker list must never enter the filtered
        // snapshot: the claim list is the load-bearing whitelist, not the
        // prepare-time one-identity-per-miner shape (which operator
        // topology commands can invalidate at runtime).
        let claim = AccessClaim {
            schema: ACCESS_CLAIM_SCHEMA.to_string(),
            run_id: RunId::new("run-whitelist").unwrap(),
            principal: "id-alice".to_string(),
            token: generate_token(),
            actor: ActorId::trusted("user/id-alice"),
            may: vec!["view_own_status".to_string()],
            user_id: Some("id-alice".to_string()),
            pool_id: None,
            identity_id: None,
            miner_id: Some(MinerId::new("miner-1").unwrap()),
            worker_ids: Some(vec![WorkerId::new("w-alice-1").unwrap()]),
        };
        let snapshot = Snapshot {
            run_id: RunId::new("run-whitelist").unwrap(),
            cursor: 0,
            retained_from: 0,
            resync_required: false,
            snapshot_cursor: 0,
            generated_at: 0,
            entities: vec![
                crate::evidence::EntitySnapshot {
                    kind: "worker".to_string(),
                    id: "w-alice-1".to_string(),
                    status: "running".to_string(),
                    detail: String::new(),
                    miner_id: Some(MinerId::new("miner-1").unwrap()),
                },
                crate::evidence::EntitySnapshot {
                    kind: "worker".to_string(),
                    id: "w-foreign-9".to_string(),
                    status: "running".to_string(),
                    detail: String::new(),
                    miner_id: Some(MinerId::new("miner-1").unwrap()),
                },
            ],
            watermarks: Vec::new(),
            gaps: Vec::new(),
            evidence: crate::evidence::EvidenceSummary {
                total_events: 0,
                counts: BTreeMap::new(),
                gap_count: 0,
                last_event_id: 0,
            },
        };
        let view = user_snapshot(&claim, &snapshot);
        let workers = view["workers"].as_array().unwrap();
        assert_eq!(workers.len(), 1, "{view}");
        assert_eq!(workers[0]["id"], "w-alice-1");
        let text = view.to_string();
        assert!(!text.contains("w-foreign-9"), "{text}");
    }

    #[test]
    fn relay_asset_list_is_closed() {
        assert_eq!(
            relay_asset_content_type("index.html"),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(relay_asset_content_type("status"), Some("application/json"));
        assert!(relay_asset_content_type("../../etc/passwd").is_none());
        assert!(relay_asset_content_type("app.js").is_none());
        assert!(relay_asset_content_type("index.html.bak").is_none());
    }

    #[test]
    fn unavailable_kinds_are_the_closed_t0_list() {
        assert!(is_unavailable_kind("send_user_payment"));
        assert!(is_unavailable_kind("bootstrap_fund"));
        assert!(!is_unavailable_kind("pause_workers"));
        assert!(!is_unavailable_kind("stop_run"));
    }
}
