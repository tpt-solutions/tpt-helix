//! Capability Broker — the enforcement core behind G3 (Capability Security).
//!
//! Design principles (spec §5.1):
//! * **No ambient authority.** A module has no access to anything not
//!   explicitly granted. Every capability-aware host operation consults the
//!   broker first.
//! * **User grants, not site requests.** The user decides what an app receives
//!   via the grant/deny/modify prompt flow (§5.3).
//! * **Fine-grained, revocable.** Grants are per-app, per-resource and can be
//!   revoked at runtime; revocation immediately denies further use.
//! * **Composable.** A grant may be delegated to another module, producing a
//!   child grant whose scope is bounded by (and revocable through) its parent.
//! * **Trap/abort on exceed.** Any operation that exceeds the granted
//!   capabilities fails with a [`CapabilityError`]; the runtime maps a revoked
//!   or out-of-scope access to a trap/abort of the offending module.
//!
//! The broker is deliberately free of WIT/tpt-wasm types so it can be unit
//! tested in isolation and shared by both the stateless `RuntimeStub` and the
//! stateful tpt-wasm `Host`.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Identifies a running application / module instance.
pub type AppId = String;

/// Monotonic broker-unique id for a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GrantId(pub u64);

/// An unforgeable, opaque capability token handed to a guest module.
///
/// The token resolves to exactly one grant inside one broker instance. Cloning
/// the token does **not** widen scope; widening requires [`CapabilityBroker::delegate`]
/// so the broker can re-scope and later revoke the delegation independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CapabilityToken(pub u128);

/// A host allow-list pattern for `network` capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostPattern {
    /// Match exactly this host (case-insensitive), e.g. `api.example.com`.
    Exact(String),
    /// Match any host ending with this suffix, e.g. `.example.com` matches
    /// `api.example.com` and `cdn.example.com` but not `example.com.evil.test`.
    Suffix(String),
    /// Match any host.
    Any,
}

impl HostPattern {
    pub fn matches(&self, host: &str) -> bool {
        match self {
            HostPattern::Any => true,
            HostPattern::Exact(e) => e.eq_ignore_ascii_case(host),
            HostPattern::Suffix(s) => {
                let s = s.trim_start_matches('.').to_ascii_lowercase();
                // An empty suffix would otherwise match every dotted host.
                if s.is_empty() {
                    return false;
                }
                let host = host.to_ascii_lowercase();
                host == s || host.ends_with(&format!(".{s}"))
            }
        }
    }
}

/// Scope of a `storage` capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageScope {
    /// Read/write any key.
    Global,
    /// Read/write only keys under this namespace prefix (e.g. `app:notes:`).
    Namespace(String),
}

/// Scope of a `dom` capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomScope {
    /// Full DOM access (create/modify/append anywhere).
    Full,
    /// Modify only the subtree rooted at this element id.
    Subtree(u64),
}

/// A single capability the broker can grant, scope, revoke, and delegate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    /// Network access constrained to a set of host patterns.
    Network { hosts: Vec<HostPattern> },
    /// Key/value storage access scoped to a namespace.
    Storage { scope: StorageScope },
    /// DOM access scoped to a subtree (or full).
    Dom { scope: DomScope },
    /// Media playback bounded by an optional maximum resolution.
    Media { max_resolution: Option<(u32, u32)> },
}

/// Actionable remediation for a denied `cap`, for the capability-denial audit
/// log: what the app's manifest or the host grant needs to change.
pub fn remediation_hint(cap: &Capability) -> &'static str {
    match cap {
        Capability::Network { .. } => {
            "declare the target host under `network` in the app manifest, or grant it with a host pattern that covers the request URL"
        }
        Capability::Storage { .. } => {
            "declare `storage` in the app manifest with a namespace prefix covering the key, or grant that namespace to the app"
        }
        Capability::Dom {
            scope: DomScope::Full,
        } => {
            "creating elements needs a `dom` grant with full scope; grant `dom` with DomScope::Full, or limit the app to subtree edits"
        }
        Capability::Dom { .. } => {
            "the target element must lie inside a `dom` subtree the app was granted; grant that subtree root (or a full-scope `dom` grant)"
        }
        Capability::Media { .. } => {
            "the requested video resolution exceeds the `media` grant; lower the resolution or grant a larger max_resolution"
        }
    }
}

/// Outcome of a capability check or operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityError {
    /// The module attempted an operation it was never granted. Deny path:
    /// surfaced to the guest as a `result<_, string>` error.
    Denied { app: AppId, capability: Capability },
    /// A previously-valid grant was revoked (or expired) and the operation
    /// must trap/abort the module. Fatal path.
    Revoked { grant: GrantId, reason: String },
    /// The token presented does not resolve to any live grant.
    InvalidToken(CapabilityToken),
}

impl std::fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapabilityError::Denied { app, capability } => {
                write!(f, "capability denied for {app}: {capability:?}")
            }
            CapabilityError::Revoked { grant, reason } => {
                write!(f, "capability grant {:#?} revoked: {reason}", grant.0)
            }
            CapabilityError::InvalidToken(t) => write!(f, "invalid capability token {:#x}", t.0),
        }
    }
}

impl std::error::Error for CapabilityError {}

/// A granted capability tracked by the broker.
#[derive(Debug, Clone)]
pub struct Grant {
    pub id: GrantId,
    pub app: AppId,
    pub capability: Capability,
    pub revoked: bool,
    pub issued_at: u64,
    pub expires_at: Option<u64>,
    /// Set when this grant was produced by delegating `parent`.
    pub parent: Option<GrantId>,
}

impl Grant {
    pub fn is_expired(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|e| now >= e)
    }
}

/// Decision returned by a [`GrantPrompter`] for a capability request (§5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantDecision {
    /// Grant exactly what was requested.
    Allow,
    /// Deny the request entirely.
    Deny,
    /// Grant a narrowed version of the request (user modified scope).
    Modify(Capability),
}

/// User-facing prompt flow: an app declares a capability, the runtime asks the
/// user, who approves / denies / modifies. `ConsolePrompter` is the interactive
/// implementation; tests use deterministic prompters.
pub trait GrantPrompter {
    fn request(&self, app: &AppId, requested: &Capability) -> GrantDecision;
}

/// Automatically allows every request (used in tests and fully-trusted hosts).
pub struct AllowAllPrompter;
impl GrantPrompter for AllowAllPrompter {
    fn request(&self, _app: &AppId, _requested: &Capability) -> GrantDecision {
        GrantDecision::Allow
    }
}

/// Automatically denies every request.
pub struct DenyAllPrompter;
impl GrantPrompter for DenyAllPrompter {
    fn request(&self, _app: &AppId, _requested: &Capability) -> GrantDecision {
        GrantDecision::Deny
    }
}

/// The Capability Broker: grant registry + scoping + revocation + delegation.
#[derive(Debug)]
pub struct CapabilityBroker {
    next_grant: u64,
    grants: HashMap<GrantId, Grant>,
    app_grants: HashMap<AppId, Vec<GrantId>>,
    tokens: HashMap<CapabilityToken, GrantId>,
    /// parent grant id -> child grant ids (for cascading revocation).
    delegations: HashMap<GrantId, Vec<GrantId>>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Default for CapabilityBroker {
    fn default() -> Self {
        CapabilityBroker {
            next_grant: 1,
            grants: HashMap::new(),
            app_grants: HashMap::new(),
            tokens: HashMap::new(),
            delegations: HashMap::new(),
        }
    }
}

impl CapabilityBroker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Draws a fresh token from OS entropy. Tokens are looked up in a table and
    /// never derived from predictable state (clock, counters, addresses).
    ///
    /// Panics if the OS entropy source is unavailable: minting a predictable
    /// token would silently defeat the capability model, so failing closed is
    /// the only safe behavior.
    fn mint_token(&self) -> CapabilityToken {
        loop {
            let mut buf = [0u8; 16];
            getrandom::fill(&mut buf)
                .expect("OS entropy unavailable: refusing to mint capability tokens");
            let token = CapabilityToken(u128::from_le_bytes(buf));
            if !self.tokens.contains_key(&token) {
                return token;
            }
        }
    }

    fn issue(
        &mut self,
        app: AppId,
        capability: Capability,
        parent: Option<GrantId>,
    ) -> (GrantId, CapabilityToken) {
        let id = GrantId(self.next_grant);
        self.next_grant += 1;
        let grant = Grant {
            id,
            app: app.clone(),
            capability,
            revoked: false,
            issued_at: now_secs(),
            expires_at: None,
            parent,
        };
        self.grants.insert(id, grant);
        self.app_grants.entry(app).or_default().push(id);
        let token = self.mint_token();
        self.tokens.insert(token, id);
        (id, token)
    }

    /// Verifies that `id` and every ancestor in its delegation chain are live
    /// (not revoked, not expired). A child is only as live as its parent: a
    /// parent that expired (rather than being explicitly revoked) must still
    /// invalidate its delegated children.
    fn ensure_live(&self, id: GrantId, now: u64) -> Result<(), CapabilityError> {
        let mut current = Some(id);
        while let Some(gid) = current {
            let grant = self
                .grants
                .get(&gid)
                .ok_or(CapabilityError::InvalidToken(CapabilityToken(0)))?;
            if grant.revoked {
                return Err(CapabilityError::Revoked {
                    grant: gid,
                    reason: if gid == id {
                        "grant revoked".into()
                    } else {
                        "ancestor grant revoked".into()
                    },
                });
            }
            if grant.is_expired(now) {
                return Err(CapabilityError::Revoked {
                    grant: gid,
                    reason: if gid == id {
                        "grant expired".into()
                    } else {
                        "ancestor grant expired".into()
                    },
                });
            }
            current = grant.parent;
        }
        Ok(())
    }

    /// Record a grant for `app` (e.g. from a previously-resolved prompt) and
    /// return the unforgeable token the app must present on each call.
    pub fn grant(&mut self, app: AppId, capability: Capability) -> CapabilityToken {
        self.issue(app, capability, None).1
    }

    /// Grant with an explicit expiration (seconds since epoch).
    pub fn grant_until(
        &mut self,
        app: AppId,
        capability: Capability,
        expires_at: u64,
    ) -> CapabilityToken {
        let (id, token) = self.issue(app, capability, None);
        if let Some(g) = self.grants.get_mut(&id) {
            g.expires_at = Some(expires_at);
        }
        token
    }

    /// Declare → request → approve flow. Consults `prompter`, applies the
    /// decision, and returns a token on allow/modify or `Denied` on deny.
    pub fn request_capability<P: GrantPrompter>(
        &mut self,
        app: &AppId,
        requested: Capability,
        prompter: &P,
    ) -> Result<CapabilityToken, CapabilityError> {
        let denied = || CapabilityError::Denied {
            app: app.clone(),
            capability: requested.clone(),
        };
        // A request that covers nothing (e.g. an empty host list) is never grantable.
        if is_empty_request(&requested) {
            return Err(denied());
        }
        match prompter.request(app, &requested) {
            GrantDecision::Deny => Err(denied()),
            GrantDecision::Allow => Ok(self.grant(app.clone(), requested)),
            // A prompter may only narrow the request, never widen it: the
            // modified capability is intersected with what was asked for.
            GrantDecision::Modify(narrowed) => {
                let scoped = intersect(&requested, &narrowed)
                    .filter(|c| !is_empty_request(c))
                    .ok_or_else(denied)?;
                Ok(self.grant(app.clone(), scoped))
            }
        }
    }

    /// Revoke a grant by id. Cascades to all delegations rooted at it.
    /// Returns `true` if the grant (or any descendant) was live and is now
    /// revoked.
    pub fn revoke(&mut self, id: GrantId) -> bool {
        let mut to_revoke = vec![id];
        let mut i = 0;
        while i < to_revoke.len() {
            if let Some(children) = self.delegations.get(&to_revoke[i]).cloned() {
                to_revoke.extend(children);
            }
            i += 1;
        }
        let mut any = false;
        for gid in to_revoke {
            if let Some(g) = self.grants.get_mut(&gid)
                && !g.revoked
            {
                g.revoked = true;
                any = true;
            }
        }
        any
    }

    /// Revoke every grant held by an app (e.g. on module teardown).
    pub fn revoke_app(&mut self, app: &AppId) -> usize {
        let ids = self.app_grants.get(app).cloned().unwrap_or_default();
        ids.iter().filter(|id| self.revoke(**id)).count()
    }

    /// Delegate a child capability from a parent token. The child's effective
    /// scope is the intersection of `parent`'s capability and `child`; the
    /// child token is independently revocable (and revoking the parent also
    /// revokes the child).
    pub fn delegate(
        &mut self,
        parent_token: CapabilityToken,
        to_app: AppId,
        child: Capability,
    ) -> Result<CapabilityToken, CapabilityError> {
        let parent_id = *self
            .tokens
            .get(&parent_token)
            .ok_or(CapabilityError::InvalidToken(parent_token))?;
        let parent = self
            .grants
            .get(&parent_id)
            .ok_or(CapabilityError::InvalidToken(parent_token))?;
        self.ensure_live(parent_id, now_secs())?;
        let scoped =
            intersect(&parent.capability, &child).ok_or_else(|| CapabilityError::Denied {
                app: to_app.clone(),
                capability: child,
            })?;
        let (child_id, child_token) = self.issue(to_app, scoped, Some(parent_id));
        self.delegations
            .entry(parent_id)
            .or_default()
            .push(child_id);
        Ok(child_token)
    }

    /// Resolve a token to its grant id (if live).
    pub fn resolve(&self, token: CapabilityToken) -> Option<GrantId> {
        self.tokens.get(&token).copied()
    }

    /// Check whether `token` authorizes `requested`. Returns the matched grant
    /// id on success, or a [`CapabilityError`] describing the failure.
    pub fn check(
        &self,
        token: CapabilityToken,
        requested: &Capability,
    ) -> Result<GrantId, CapabilityError> {
        let id = *self
            .tokens
            .get(&token)
            .ok_or(CapabilityError::InvalidToken(token))?;
        let grant = self
            .grants
            .get(&id)
            .ok_or(CapabilityError::InvalidToken(token))?;
        self.ensure_live(id, now_secs())?;
        match contains(&grant.capability, requested) {
            true => Ok(id),
            false => Err(CapabilityError::Denied {
                app: grant.app.clone(),
                capability: requested.clone(),
            }),
        }
    }

    /// List live (non-revoked, non-expired) grants for an app.
    pub fn list_grants(&self, app: &AppId) -> Vec<&Grant> {
        let now = now_secs();
        self.app_grants
            .get(app)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.grants.get(id))
                    .filter(|g| !g.revoked && !g.is_expired(now))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn grant_count(&self) -> usize {
        self.grants.len()
    }
}

/// Is `requested` a capability that can never be satisfied (covers nothing)?
fn is_empty_request(requested: &Capability) -> bool {
    match requested {
        Capability::Network { hosts } => hosts.is_empty(),
        Capability::Storage {
            scope: StorageScope::Namespace(n),
        } => n.is_empty(),
        _ => false,
    }
}

/// Does `grant` (the authorized capability) cover `requested` (the operation)?
fn contains(grant: &Capability, requested: &Capability) -> bool {
    if is_empty_request(requested) {
        return false;
    }
    match (grant, requested) {
        (Capability::Network { hosts: gh }, Capability::Network { hosts: rh }) => {
            !rh.iter().any(|p| !gh.iter().any(|g| pattern_covers(g, p)))
        }
        (Capability::Storage { scope: gs }, Capability::Storage { scope: rs }) => {
            storage_contains(gs, rs)
        }
        (Capability::Dom { scope: gs }, Capability::Dom { scope: rs }) => dom_contains(gs, rs),
        (Capability::Media { max_resolution: gm }, Capability::Media { max_resolution: rm }) => {
            match (gm, rm) {
                (Some((gw, gh)), Some((rw, rh))) => rw <= gw && rh <= gh,
                // A bounded grant never authorizes an unbounded request.
                (Some(_), None) => false,
                (None, _) => true,
            }
        }
        _ => false,
    }
}

/// A pattern `g` covers pattern `p` if every host matched by `p` is also
/// matched by `g`.
///
/// An exact grant covers only exact requests: `Exact("example.com")` does not
/// authorize `Suffix(".example.com")`, which would also reach every subdomain.
fn pattern_covers(g: &HostPattern, p: &HostPattern) -> bool {
    match (g, p) {
        (HostPattern::Any, _) => true,
        (HostPattern::Exact(ge), HostPattern::Exact(pe)) => ge.eq_ignore_ascii_case(pe),
        (HostPattern::Exact(_), HostPattern::Suffix(_)) => false,
        (HostPattern::Exact(_), HostPattern::Any) => false,
        (HostPattern::Suffix(gs), HostPattern::Suffix(ps)) => {
            let gs = gs.trim_start_matches('.').to_ascii_lowercase();
            let ps = ps.trim_start_matches('.').to_ascii_lowercase();
            !gs.is_empty() && (ps == gs || ps.ends_with(&format!(".{gs}")))
        }
        (HostPattern::Suffix(gs), HostPattern::Exact(pe)) => {
            HostPattern::Suffix(gs.clone()).matches(pe)
        }
        (HostPattern::Suffix(_), HostPattern::Any) => false,
    }
}

/// Namespace delimiters: a grant on `app` covers `app:x` and `app/x`, but not
/// `apple:x`.
const NAMESPACE_DELIMITERS: [char; 2] = [':', '/'];

fn storage_contains(g: &StorageScope, r: &StorageScope) -> bool {
    match (g, r) {
        (StorageScope::Global, _) => true,
        (StorageScope::Namespace(gn), StorageScope::Namespace(rn)) => {
            if gn.is_empty() {
                return false;
            }
            if rn == gn {
                return true;
            }
            match rn.strip_prefix(gn.as_str()) {
                Some(rest) => {
                    gn.ends_with(NAMESPACE_DELIMITERS) || rest.starts_with(NAMESPACE_DELIMITERS)
                }
                None => false,
            }
        }
        _ => false,
    }
}

fn dom_contains(g: &DomScope, r: &DomScope) -> bool {
    match (g, r) {
        (DomScope::Full, _) => true,
        (DomScope::Subtree(gr), DomScope::Subtree(rr)) => gr == rr,
        _ => false,
    }
}

/// Intersect two capabilities for delegation: the child may be no wider than
/// the parent.
fn intersect(parent: &Capability, child: &Capability) -> Option<Capability> {
    match (parent, child) {
        (Capability::Network { hosts: ph }, Capability::Network { hosts: ch }) => {
            let mut hosts = Vec::new();
            for cp in ch {
                if ph.iter().any(|pp| pattern_covers(pp, cp)) {
                    hosts.push(cp.clone());
                }
            }
            if hosts.is_empty() {
                None
            } else {
                Some(Capability::Network { hosts })
            }
        }
        (Capability::Storage { scope: ps }, Capability::Storage { scope: rs }) => {
            storage_contains(ps, rs).then(|| child.clone())
        }
        (Capability::Dom { scope: ps }, Capability::Dom { scope: rs }) => {
            dom_contains(ps, rs).then(|| child.clone())
        }
        (Capability::Media { max_resolution: pm }, Capability::Media { max_resolution: rm }) => {
            let max = match (pm, rm) {
                (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.min(b.1))),
                _ => pm.or(*rm),
            };
            Some(Capability::Media {
                max_resolution: max,
            })
        }
        _ => None,
    }
}

/// Extracts the lowercase host of a URL (`https://api.example.com/path` →
/// `api.example.com`), handling ports, userinfo, IPv6 literals, and
/// backslashes.
///
/// URLs with a scheme are parsed by the `url` crate, which applies the WHATWG
/// rules. A scheme-less authority (`api.example.com:8443/x`) is parsed by hand
/// with the same delimiters. Returns an empty string when no host can be
/// determined; an empty host matches no `Exact`/`Suffix` pattern, so the
/// request fails closed.
pub fn host_of(input: &str) -> String {
    // Only inputs with an explicit `://` are URLs. Without it, `url` would read
    // `host:port` as `scheme:opaque-path` and yield no host.
    if input.contains("://") {
        return match url::Url::parse(input) {
            Ok(parsed) => match parsed.host() {
                Some(url::Host::Domain(d)) => d.to_ascii_lowercase(),
                Some(url::Host::Ipv4(a)) => a.to_string(),
                Some(url::Host::Ipv6(a)) => a.to_string(),
                None => String::new(),
            },
            Err(_) => String::new(),
        };
    }
    // Scheme-less: take the authority up to the first path/query/fragment
    // delimiter (a backslash also ends it, as in special-scheme URLs).
    let authority = input
        .trim_start_matches("//")
        .split(['/', '\\', '?', '#'])
        .next()
        .unwrap_or("");
    // Drop userinfo: everything up to the last '@'.
    let hostport = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(rest) = hostport.strip_prefix('[') {
        // IPv6 literal: `[addr]` or `[addr]:port`.
        rest.split(']').next().unwrap_or("")
    } else {
        // Strip a trailing `:port`; a bare host has exactly one ':' at most
        // only when it is a port (multiple colons are not a valid hostname).
        match hostport.matches(':').count() {
            0 => hostport,
            1 => hostport.split(':').next().unwrap_or(""),
            _ => "",
        }
    };
    host.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network(hosts: &[&str]) -> Capability {
        Capability::Network {
            hosts: hosts
                .iter()
                .map(|h| HostPattern::Exact(h.to_string()))
                .collect(),
        }
    }

    #[test]
    fn grant_allows_matching_request() {
        let mut b = CapabilityBroker::new();
        let tok = b.grant("app".into(), network(&["api.example.com"]));
        assert!(b.check(tok, &network(&["api.example.com"])).is_ok());
    }

    #[test]
    fn deny_unregistered_host() {
        let mut b = CapabilityBroker::new();
        let tok = b.grant("app".into(), network(&["api.example.com"]));
        let err = b.check(tok, &network(&["evil.test"])).unwrap_err();
        assert!(matches!(err, CapabilityError::Denied { .. }));
    }

    #[test]
    fn suffix_pattern_covers_subdomain() {
        let mut b = CapabilityBroker::new();
        let tok = b.grant(
            "app".into(),
            Capability::Network {
                hosts: vec![HostPattern::Suffix(".example.com".into())],
            },
        );
        assert!(b.check(tok, &network(&["api.example.com"])).is_ok());
        assert!(b.check(tok, &network(&["cdn.example.com"])).is_ok());
        assert!(b.check(tok, &network(&["example.com.evil.test"])).is_err());
    }

    #[test]
    fn revocation_denies_after_revoke() {
        let mut b = CapabilityBroker::new();
        let id = {
            let tok = b.grant("app".into(), network(&["api.example.com"]));
            b.resolve(tok).unwrap()
        };
        assert!(b.revoke(id));
        let tok = *b.tokens.iter().find(|(_, g)| **g == id).unwrap().0;
        let err = b.check(tok, &network(&["api.example.com"])).unwrap_err();
        assert!(matches!(err, CapabilityError::Revoked { .. }));
    }

    #[test]
    fn revoke_app_cascades() {
        let mut b = CapabilityBroker::new();
        b.grant("app".into(), network(&["api.example.com"]));
        b.grant(
            "app".into(),
            Capability::Storage {
                scope: StorageScope::Global,
            },
        );
        assert_eq!(b.revoke_app(&"app".into()), 2);
        assert!(b.list_grants(&"app".into()).is_empty());
    }

    #[test]
    fn delegation_is_scoped_and_revocable_via_parent() {
        let mut b = CapabilityBroker::new();
        let parent = b.grant(
            "parent".into(),
            Capability::Network {
                hosts: vec![HostPattern::Suffix(".example.com".into())],
            },
        );
        let child = b
            .delegate(parent, "child".into(), network(&["api.example.com"]))
            .unwrap();
        assert!(b.check(child, &network(&["api.example.com"])).is_ok());
        // child cannot exceed parent scope
        assert!(
            b.delegate(parent, "child".into(), network(&["other.test"]))
                .is_err()
        );

        // revoking the parent revokes the child
        let pid = b.resolve(parent).unwrap();
        b.revoke(pid);
        assert!(matches!(
            b.check(child, &network(&["api.example.com"])),
            Err(CapabilityError::Revoked { .. })
        ));
    }

    #[test]
    fn storage_namespace_scoping() {
        let mut b = CapabilityBroker::new();
        let tok = b.grant(
            "app".into(),
            Capability::Storage {
                scope: StorageScope::Namespace("app:notes:".into()),
            },
        );
        assert!(
            b.check(
                tok,
                &Capability::Storage {
                    scope: StorageScope::Namespace("app:notes:2024".into())
                }
            )
            .is_ok()
        );
        assert!(
            b.check(
                tok,
                &Capability::Storage {
                    scope: StorageScope::Namespace("other:".into())
                }
            )
            .is_err()
        );
    }

    #[test]
    fn media_resolution_cap() {
        let mut b = CapabilityBroker::new();
        let tok = b.grant(
            "app".into(),
            Capability::Media {
                max_resolution: Some((1280, 720)),
            },
        );
        assert!(
            b.check(
                tok,
                &Capability::Media {
                    max_resolution: Some((640, 480))
                }
            )
            .is_ok()
        );
        assert!(
            b.check(
                tok,
                &Capability::Media {
                    max_resolution: Some((1920, 1080))
                }
            )
            .is_err()
        );
    }

    #[test]
    fn prompt_flow_allows_and_denies() {
        let mut b = CapabilityBroker::new();
        let tok = b
            .request_capability(
                &"app".into(),
                network(&["api.example.com"]),
                &AllowAllPrompter,
            )
            .unwrap();
        assert!(b.check(tok, &network(&["api.example.com"])).is_ok());

        let mut b2 = CapabilityBroker::new();
        let err = b2
            .request_capability(
                &"app".into(),
                network(&["api.example.com"]),
                &DenyAllPrompter,
            )
            .unwrap_err();
        assert!(matches!(err, CapabilityError::Denied { .. }));
    }

    #[test]
    fn prompt_flow_modify_narrows_scope() {
        struct Narrow;
        impl GrantPrompter for Narrow {
            fn request(&self, _app: &AppId, _requested: &Capability) -> GrantDecision {
                GrantDecision::Modify(Capability::Storage {
                    scope: StorageScope::Namespace("app:".into()),
                })
            }
        }
        let mut b = CapabilityBroker::new();
        let tok = b
            .request_capability(
                &"app".into(),
                Capability::Storage {
                    scope: StorageScope::Global,
                },
                &Narrow,
            )
            .unwrap();
        assert!(
            b.check(
                tok,
                &Capability::Storage {
                    scope: StorageScope::Namespace("app:x".into())
                }
            )
            .is_ok()
        );
        assert!(
            b.check(
                tok,
                &Capability::Storage {
                    scope: StorageScope::Global
                }
            )
            .is_err()
        );
    }

    #[test]
    fn host_of_parses_urls() {
        assert_eq!(
            host_of("https://api.example.com/path?q=1"),
            "api.example.com"
        );
        assert_eq!(host_of("http://cdn.example.com"), "cdn.example.com");
        assert_eq!(host_of("api.example.com"), "api.example.com");
    }

    #[test]
    fn host_of_handles_ports_userinfo_case_and_ipv6() {
        assert_eq!(host_of("https://API.Example.COM:8443/x"), "api.example.com");
        assert_eq!(
            host_of("https://user:pw@api.example.com/x"),
            "api.example.com"
        );
        assert_eq!(
            host_of("https://evil.test@api.example.com/x"),
            "api.example.com"
        );
        assert_eq!(host_of("https://[::1]:8080/x"), "::1");
        assert_eq!(host_of("api.example.com:8443/x"), "api.example.com");
        assert_eq!(host_of("api.example.com:8443"), "api.example.com");
        // Backslash ends the authority, as in special-scheme URLs.
        assert_eq!(
            host_of("https://api.example.com\\evil.test/x"),
            "api.example.com"
        );
        // Unparseable / hostless input fails closed with an empty host.
        assert_eq!(host_of(""), "");
        assert_eq!(host_of("https://"), "");
    }

    #[test]
    fn exact_grant_does_not_cover_suffix_request() {
        let mut b = CapabilityBroker::new();
        let tok = b.grant("app".into(), network(&["example.com"]));
        let suffix = Capability::Network {
            hosts: vec![HostPattern::Suffix(".example.com".into())],
        };
        assert!(b.check(tok, &suffix).is_err());
        assert!(b.check(tok, &network(&["example.com"])).is_ok());
        assert!(b.check(tok, &network(&["api.example.com"])).is_err());
    }

    #[test]
    fn empty_host_list_is_never_granted_or_authorized() {
        let mut b = CapabilityBroker::new();
        let tok = b.grant("app".into(), network(&["api.example.com"]));
        assert!(b.check(tok, &network(&[])).is_err());

        let err = b
            .request_capability(&"app".into(), network(&[]), &AllowAllPrompter)
            .unwrap_err();
        assert!(matches!(err, CapabilityError::Denied { .. }));
    }

    #[test]
    fn bounded_media_grant_rejects_unbounded_request() {
        let mut b = CapabilityBroker::new();
        let tok = b.grant(
            "app".into(),
            Capability::Media {
                max_resolution: Some((1280, 720)),
            },
        );
        assert!(
            b.check(
                tok,
                &Capability::Media {
                    max_resolution: None
                }
            )
            .is_err()
        );
        // An unbounded grant still covers bounded requests.
        let open = b.grant(
            "app".into(),
            Capability::Media {
                max_resolution: None,
            },
        );
        assert!(
            b.check(
                open,
                &Capability::Media {
                    max_resolution: Some((3840, 2160))
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn namespace_prefix_must_be_delimited() {
        let mut b = CapabilityBroker::new();
        let tok = b.grant(
            "app".into(),
            Capability::Storage {
                scope: StorageScope::Namespace("app".into()),
            },
        );
        let req = |ns: &str| Capability::Storage {
            scope: StorageScope::Namespace(ns.into()),
        };
        assert!(b.check(tok, &req("app:x")).is_ok());
        assert!(b.check(tok, &req("app")).is_ok());
        assert!(b.check(tok, &req("apple:x")).is_err());
    }

    #[test]
    fn empty_namespace_grant_covers_nothing() {
        let mut b = CapabilityBroker::new();
        let tok = b.grant(
            "app".into(),
            Capability::Storage {
                scope: StorageScope::Namespace(String::new()),
            },
        );
        assert!(
            b.check(
                tok,
                &Capability::Storage {
                    scope: StorageScope::Namespace("anything".into())
                }
            )
            .is_err()
        );
    }

    #[test]
    fn modify_cannot_widen_requested_scope() {
        struct Widen;
        impl GrantPrompter for Widen {
            fn request(&self, _app: &AppId, _requested: &Capability) -> GrantDecision {
                GrantDecision::Modify(Capability::Storage {
                    scope: StorageScope::Global,
                })
            }
        }
        let mut b = CapabilityBroker::new();
        let err = b
            .request_capability(
                &"app".into(),
                Capability::Storage {
                    scope: StorageScope::Namespace("app:".into()),
                },
                &Widen,
            )
            .unwrap_err();
        assert!(matches!(err, CapabilityError::Denied { .. }));
    }

    #[test]
    fn parent_expiry_invalidates_delegated_child() {
        let mut b = CapabilityBroker::new();
        let parent = b.grant_until(
            "parent".into(),
            Capability::Network {
                hosts: vec![HostPattern::Any],
            },
            // Already expired.
            1,
        );
        // Delegating from an expired parent is refused outright.
        assert!(
            b.delegate(parent, "child".into(), network(&["api.example.com"]))
                .is_err()
        );

        // A child created while the parent was live is invalidated when the
        // parent later expires (no explicit revoke).
        let mut b = CapabilityBroker::new();
        let parent = b.grant_until(
            "parent".into(),
            Capability::Network {
                hosts: vec![HostPattern::Any],
            },
            now_secs() + 3600,
        );
        let child = b
            .delegate(parent, "child".into(), network(&["api.example.com"]))
            .unwrap();
        assert!(b.check(child, &network(&["api.example.com"])).is_ok());
        let pid = b.resolve(parent).unwrap();
        b.grants.get_mut(&pid).unwrap().expires_at = Some(1);
        assert!(matches!(
            b.check(child, &network(&["api.example.com"])),
            Err(CapabilityError::Revoked { .. })
        ));
    }

    #[test]
    fn tokens_are_unique_and_unpredictable_across_brokers() {
        let mut a = CapabilityBroker::new();
        let mut b = CapabilityBroker::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            assert!(seen.insert(a.grant("app".into(), network(&["x.test"]))));
        }
        // Independent brokers do not produce the same token stream.
        let ta = a.grant("app".into(), network(&["x.test"]));
        let tb = b.grant("app".into(), network(&["x.test"]));
        assert_ne!(ta, tb);
    }
}
