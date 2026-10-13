//! Background window leases: a port of aevra's `BackgroundDesktopState`
//! (`packages/desktop/src/background-state.ts`).
//!
//! A lease reserves one window for one owner (an MCP connection) and binds the
//! refs of the owner's latest background snapshot to native element handles.
//! Every error is a [`DesktopError`] with aevra's code.

use desktop_core::{DesktopError, VerifiedWindowHost, WindowInstance};
use desktop_policy::gate::canonical_path;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const TTL_MS: u64 = 60_000;
pub const MAX_LEASES_PER_OWNER: usize = 8;
pub const MAX_LEASES_TOTAL: usize = 32;
pub const MAX_NODES_PER_SNAPSHOT: usize = 5_000;

pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;
pub type IdGen = Arc<dyn Fn() -> String + Send + Sync>;

/// One node of a bound snapshot: the native handle and whether the node is a
/// password, OTP or payment field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundNode {
    pub handle: String,
    pub sensitive: Option<&'static str>,
}

#[derive(Debug, Clone)]
struct Snapshot {
    snapshot_id: String,
    epoch: u64,
    nodes: HashMap<String, BoundNode>,
}

#[derive(Debug, Clone)]
struct Lease {
    lease_id: String,
    owner: String,
    window: WindowInstance,
    host: Option<VerifiedWindowHost>,
    app_id: String,
    epoch: u64,
    expires_at: u64,
    /// The snapshot refs currently resolve against.
    active: Option<Snapshot>,
    /// The latest snapshot the helper still holds for this lease. It survives
    /// `invalidate_snapshot` and `suspend`, because invalidating a ref table
    /// frees nothing natively, so `release` must hand it back to be freed.
    native_snapshot: Option<String>,
    suspended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acquired {
    pub lease_id: String,
    pub expires_at_ms: u64,
}

impl Acquired {
    pub fn expires_at(&self) -> String {
        desktop_policy::activity::rfc3339(
            (self.expires_at_ms / 1000) as i64,
            (self.expires_at_ms % 1000) as u32,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bound {
    pub refs: Vec<String>,
    /// The helper snapshot this bind replaced; the caller frees it.
    pub superseded: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub handle: String,
    pub snapshot_id: String,
    pub instance: WindowInstance,
    pub host: Option<VerifiedWindowHost>,
    pub app_id: String,
    pub lease_id: String,
    pub sensitive: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Released {
    /// The native snapshot to free, if the lease ever bound one.
    pub snapshot_id: Option<String>,
}

#[derive(Default)]
struct State {
    leases: HashMap<String, Lease>,
    next_ref: u64,
    /// Native snapshots of leases that expired on their own.
    pending_free: Vec<String>,
}

pub struct Leases {
    state: Mutex<State>,
    now: Clock,
    new_id: IdGen,
}

fn system_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn scope_key(w: &WindowInstance) -> String {
    format!("{}:{}:{}", w.window_id, w.process_id, w.process_started_at)
}

fn host_key(h: Option<&VerifiedWindowHost>) -> String {
    match h {
        None => String::new(),
        Some(h) => format!(
            "{}:{}:{}:{}",
            h.instance.window_id,
            h.instance.process_id,
            h.instance.process_started_at,
            canonical_path(&h.executable_path)
        ),
    }
}

fn busy(message: &str, retry_after_ms: u64) -> DesktopError {
    DesktopError::new("DESKTOP_WINDOW_BUSY", message)
        .with_details(json!({"retryAfterMs": retry_after_ms}))
}

impl Default for Leases {
    fn default() -> Self {
        Self::new()
    }
}

impl Leases {
    pub fn new() -> Self {
        Self::with(
            Arc::new(system_now),
            Arc::new(|| uuid::Uuid::new_v4().to_string()),
        )
    }

    /// For tests: an injected clock (milliseconds) and id generator.
    pub fn with(now: Clock, new_id: IdGen) -> Self {
        Self {
            state: Mutex::new(State {
                next_ref: 1,
                ..State::default()
            }),
            now,
            new_id,
        }
    }

    fn prune(&self, s: &mut State) {
        let now = (self.now)();
        let expired: Vec<String> = s
            .leases
            .iter()
            .filter(|(_, l)| l.expires_at <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            if let Some(lease) = s.leases.remove(&id) {
                s.pending_free.extend(lease.native_snapshot);
            }
        }
    }

    /// Native snapshots of leases that expired since the last call. The caller
    /// frees them through the backend.
    pub fn take_pending_free(&self) -> Vec<String> {
        let mut s = self.state.lock().unwrap();
        self.prune(&mut s);
        std::mem::take(&mut s.pending_free)
    }

    pub fn acquire(
        &self,
        owner: &str,
        window: &WindowInstance,
        epoch: u64,
        host: Option<&VerifiedWindowHost>,
        app_id: &str,
    ) -> Result<Acquired, DesktopError> {
        let mut s = self.state.lock().unwrap();
        self.prune(&mut s);
        let now = (self.now)();
        let key = scope_key(window);

        if let Some(existing) = s.leases.values_mut().find(|l| scope_key(&l.window) == key) {
            if host_key(existing.host.as_ref()) != host_key(host) {
                return Err(DesktopError::new(
                    "DESKTOP_TARGET_CHANGED",
                    "Verified host application changed for the leased target window",
                ));
            }
            if existing.owner != owner {
                let retry = (existing.expires_at.saturating_sub(now)).max(1_000);
                return Err(busy("Window is owned by another session", retry));
            }
            existing.expires_at = now + TTL_MS;
            existing.epoch = epoch;
            return Ok(Acquired {
                lease_id: existing.lease_id.clone(),
                expires_at_ms: existing.expires_at,
            });
        }

        if s.leases.len() >= MAX_LEASES_TOTAL {
            return Err(busy("Host window lease limit reached", 5_000));
        }
        if s.leases.values().filter(|l| l.owner == owner).count() >= MAX_LEASES_PER_OWNER {
            return Err(busy(
                "Session window lease quota exceeded (maximum 8 active window leases)",
                5_000,
            ));
        }

        let lease_id = (self.new_id)();
        let expires_at = now + TTL_MS;
        s.leases.insert(
            lease_id.clone(),
            Lease {
                lease_id: lease_id.clone(),
                owner: owner.to_string(),
                window: window.clone(),
                host: host.cloned(),
                app_id: app_id.to_string(),
                epoch,
                expires_at,
                active: None,
                native_snapshot: None,
                suspended: false,
            },
        );
        Ok(Acquired {
            lease_id,
            expires_at_ms: expires_at,
        })
    }

    /// Binds `nodes` to fresh `ref_bg_<epoch>_<n>` refs under `snapshot_id`.
    pub fn bind(
        &self,
        owner: &str,
        lease_id: &str,
        snapshot_id: &str,
        epoch: u64,
        nodes: Vec<BoundNode>,
    ) -> Result<Bound, DesktopError> {
        let mut s = self.state.lock().unwrap();
        self.prune(&mut s);
        if s.leases.get(lease_id).is_none_or(|l| l.owner != owner) {
            return Err(DesktopError::new(
                "DESKTOP_LEASE_EXPIRED",
                "Window lease not found or expired",
            ));
        }
        if nodes.len() > MAX_NODES_PER_SNAPSHOT {
            return Err(DesktopError::new(
                "DESKTOP_PATTERN_UNSUPPORTED",
                format!("Snapshot exceeds maximum node limit ({MAX_NODES_PER_SNAPSHOT})"),
            ));
        }
        let mut refs = Vec::with_capacity(nodes.len());
        let mut map = HashMap::with_capacity(nodes.len());
        for node in nodes {
            let r = format!("ref_bg_{epoch}_{}", s.next_ref);
            s.next_ref += 1;
            refs.push(r.clone());
            map.insert(r, node);
        }
        let lease = s.leases.get_mut(lease_id).expect("checked above");
        lease.suspended = false;
        let superseded = lease
            .native_snapshot
            .replace(snapshot_id.to_string())
            .filter(|old| old != snapshot_id);
        lease.active = Some(Snapshot {
            snapshot_id: snapshot_id.to_string(),
            epoch,
            nodes: map,
        });
        Ok(Bound { refs, superseded })
    }

    /// Checks, in aevra's order: lease and owner, suspension, expiry, helper
    /// epoch, window id, snapshot, snapshot epoch, ref. A success renews the TTL.
    pub fn resolve(
        &self,
        owner: &str,
        window_id: &str,
        lease_id: &str,
        snapshot_id: &str,
        reference: &str,
        epoch: u64,
    ) -> Result<Resolved, DesktopError> {
        let mut s = self.state.lock().unwrap();
        self.prune(&mut s);
        let now = (self.now)();
        let lease = match s.leases.get_mut(lease_id) {
            Some(l) if l.owner == owner => l,
            _ => {
                return Err(DesktopError::new(
                    "DESKTOP_LEASE_EXPIRED",
                    "Window lease expired or owner mismatch",
                ))
            }
        };
        if lease.suspended {
            return Err(DesktopError::new(
                "DESKTOP_FOCUS_CHANGED",
                "Window lease is suspended due to observed focus change; fresh background describe required",
            ));
        }
        if lease.expires_at <= now {
            return Err(DesktopError::new(
                "DESKTOP_LEASE_EXPIRED",
                "Window lease expired",
            ));
        }
        if lease.epoch != epoch {
            return Err(DesktopError::new(
                "DESKTOP_REF_STALE",
                "Helper epoch mismatch",
            ));
        }
        if lease.window.window_id != window_id {
            return Err(DesktopError::new(
                "DESKTOP_TARGET_CHANGED",
                "Window ID mismatch for lease",
            ));
        }
        let Some(active) = lease
            .active
            .as_ref()
            .filter(|a| a.snapshot_id == snapshot_id)
        else {
            return Err(DesktopError::new(
                "DESKTOP_REF_STALE",
                "Snapshot invalidated or stale",
            ));
        };
        if active.epoch != epoch {
            return Err(DesktopError::new(
                "DESKTOP_REF_STALE",
                "Snapshot helper epoch mismatch",
            ));
        }
        let Some(node) = active.nodes.get(reference).cloned() else {
            return Err(DesktopError::new(
                "DESKTOP_REF_STALE",
                format!("Node reference not found in snapshot: {reference}"),
            ));
        };
        let resolved = Resolved {
            handle: node.handle,
            snapshot_id: active.snapshot_id.clone(),
            instance: lease.window.clone(),
            host: lease.host.clone(),
            app_id: lease.app_id.clone(),
            lease_id: lease.lease_id.clone(),
            sensitive: node.sensitive,
        };
        lease.expires_at = now + TTL_MS;
        Ok(resolved)
    }

    /// Marks the lease as having seen a focus change: refs stop resolving until
    /// a fresh describe binds a new snapshot.
    pub fn suspend(&self, lease_id: &str) {
        if let Some(l) = self.state.lock().unwrap().leases.get_mut(lease_id) {
            l.suspended = true;
            l.active = None;
        }
    }

    /// Drops the ref table. The helper's snapshot is not freed; `release` returns it.
    pub fn invalidate_snapshot(&self, lease_id: &str) {
        if let Some(l) = self.state.lock().unwrap().leases.get_mut(lease_id) {
            l.active = None;
        }
    }

    /// Ends a lease. `None` when the lease is unknown, owned by someone else, or
    /// for a different window.
    pub fn release(&self, owner: &str, window_id: &str, lease_id: &str) -> Option<Released> {
        let mut s = self.state.lock().unwrap();
        self.prune(&mut s);
        let lease = s.leases.get(lease_id)?;
        if lease.owner != owner || lease.window.window_id != window_id {
            return None;
        }
        let lease = s.leases.remove(lease_id)?;
        Some(Released {
            snapshot_id: lease.native_snapshot,
        })
    }

    /// Ends a lease regardless of its owner. Used when the gate refuses an app
    /// the lease was acquired for.
    pub fn release_lease(&self, lease_id: &str) -> Option<Released> {
        let lease = self.state.lock().unwrap().leases.remove(lease_id)?;
        Some(Released {
            snapshot_id: lease.native_snapshot,
        })
    }

    pub fn is_window_leased(&self, window_id: &str) -> bool {
        let mut s = self.state.lock().unwrap();
        self.prune(&mut s);
        s.leases.values().any(|l| l.window.window_id == window_id)
    }

    /// Drops every lease. Native snapshots are dropped with the helper session.
    pub fn reset(&self) {
        let mut s = self.state.lock().unwrap();
        s.leases.clear();
        s.pending_free.clear();
    }

    /// Ends every lease and returns their native snapshots, for freeing when
    /// the session disconnects or reconnects.
    pub fn drain_all(&self) -> Vec<String> {
        let mut s = self.state.lock().unwrap();
        let mut ids = std::mem::take(&mut s.pending_free);
        ids.extend(s.leases.drain().filter_map(|(_, l)| l.native_snapshot));
        ids
    }

    /// Ends every lease of `owner` and returns their native snapshots.
    pub fn invalidate_owner(&self, owner: &str) -> Vec<String> {
        let mut s = self.state.lock().unwrap();
        let ids: Vec<String> = s
            .leases
            .iter()
            .filter(|(_, l)| l.owner == owner)
            .map(|(id, _)| id.clone())
            .collect();
        ids.into_iter()
            .filter_map(|id| s.leases.remove(&id))
            .filter_map(|l| l.native_snapshot)
            .collect()
    }

    /// Ends every lease acquired for `app_id` and returns their native
    /// snapshots. Used on config hot reload when an app is removed.
    pub fn release_for_app(&self, app_id: &str) -> Vec<String> {
        let mut s = self.state.lock().unwrap();
        let ids: Vec<String> = s
            .leases
            .iter()
            .filter(|(_, l)| l.app_id == app_id)
            .map(|(id, _)| id.clone())
            .collect();
        ids.into_iter()
            .filter_map(|id| s.leases.remove(&id))
            .filter_map(|l| l.native_snapshot)
            .collect()
    }

    /// Whether any lease for `app_id` exists. Test and diagnostics helper.
    pub fn lease_count_for_app(&self, app_id: &str) -> usize {
        self.state
            .lock()
            .unwrap()
            .leases
            .values()
            .filter(|l| l.app_id == app_id)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture {
        leases: Leases,
        clock: Arc<AtomicU64>,
    }

    fn fixture() -> Fixture {
        let clock = Arc::new(AtomicU64::new(1_000_000));
        let ids = Arc::new(AtomicU64::new(1));
        let c = Arc::clone(&clock);
        let leases = Leases::with(
            Arc::new(move || c.load(Ordering::SeqCst)),
            Arc::new(move || format!("lease-{}", ids.fetch_add(1, Ordering::SeqCst))),
        );
        Fixture { leases, clock }
    }

    fn win(id: &str) -> WindowInstance {
        WindowInstance::new(id, 10, "t0")
    }

    fn node(handle: &str) -> BoundNode {
        BoundNode {
            handle: handle.into(),
            sensitive: None,
        }
    }

    fn bound(f: &Fixture, owner: &str, window: &str, snapshot: &str) -> (String, Vec<String>) {
        let acq = f
            .leases
            .acquire(owner, &win(window), 1, None, "app_a")
            .unwrap();
        let b = f
            .leases
            .bind(
                owner,
                &acq.lease_id,
                snapshot,
                1,
                vec![node("h0"), node("h1")],
            )
            .unwrap();
        (acq.lease_id, b.refs)
    }

    #[test]
    fn a_second_owner_on_the_same_window_is_busy() {
        let f = fixture();
        f.leases
            .acquire("alice", &win("1"), 1, None, "app_a")
            .unwrap();
        let err = f
            .leases
            .acquire("bob", &win("1"), 1, None, "app_a")
            .unwrap_err();
        assert_eq!(err.code, "DESKTOP_WINDOW_BUSY");
        let retry = err.details.unwrap()["retryAfterMs"].as_u64().unwrap();
        assert!(retry >= 1_000);
    }

    #[test]
    fn the_same_owner_renews_the_lease() {
        let f = fixture();
        let first = f
            .leases
            .acquire("alice", &win("1"), 1, None, "app_a")
            .unwrap();
        f.clock.fetch_add(30_000, Ordering::SeqCst);
        let again = f
            .leases
            .acquire("alice", &win("1"), 1, None, "app_a")
            .unwrap();
        assert_eq!(first.lease_id, again.lease_id);
        assert_eq!(again.expires_at_ms, 1_000_000 + 30_000 + TTL_MS);
    }

    #[test]
    fn a_changed_host_on_a_leased_window_is_target_changed() {
        let f = fixture();
        f.leases
            .acquire("alice", &win("1"), 1, None, "app_a")
            .unwrap();
        let host = VerifiedWindowHost {
            executable_path: "C:\\h.exe".into(),
            instance: win("2"),
        };
        let err = f
            .leases
            .acquire("alice", &win("1"), 1, Some(&host), "app_a")
            .unwrap_err();
        assert_eq!(err.code, "DESKTOP_TARGET_CHANGED");
    }

    #[test]
    fn the_ninth_lease_for_one_owner_is_busy() {
        let f = fixture();
        for n in 0..8 {
            f.leases
                .acquire("alice", &win(&n.to_string()), 1, None, "app_a")
                .unwrap();
        }
        let err = f
            .leases
            .acquire("alice", &win("99"), 1, None, "app_a")
            .unwrap_err();
        assert_eq!(err.code, "DESKTOP_WINDOW_BUSY");
        assert_eq!(err.details.unwrap()["retryAfterMs"], 5_000);
    }

    #[test]
    fn the_host_wide_limit_applies() {
        let f = fixture();
        for n in 0..MAX_LEASES_TOTAL {
            f.leases
                .acquire(&format!("owner{n}"), &win(&n.to_string()), 1, None, "app_a")
                .unwrap();
        }
        let err = f
            .leases
            .acquire("late", &win("999"), 1, None, "app_a")
            .unwrap_err();
        assert_eq!(err.code, "DESKTOP_WINDOW_BUSY");
    }

    #[test]
    fn resolve_returns_the_bound_handle() {
        let f = fixture();
        let (lease, refs) = bound(&f, "alice", "1", "snap-1");
        let r = f
            .leases
            .resolve("alice", "1", &lease, "snap-1", &refs[1], 1)
            .unwrap();
        assert_eq!(r.handle, "h1");
        assert_eq!(r.snapshot_id, "snap-1");
        assert_eq!(r.app_id, "app_a");
        assert!(refs[0].starts_with("ref_bg_1_"));
    }

    #[test]
    fn resolve_after_ttl_is_lease_expired() {
        let f = fixture();
        let (lease, refs) = bound(&f, "alice", "1", "snap-1");
        f.clock.fetch_add(TTL_MS + 1, Ordering::SeqCst);
        let err = f
            .leases
            .resolve("alice", "1", &lease, "snap-1", &refs[0], 1)
            .unwrap_err();
        assert_eq!(err.code, "DESKTOP_LEASE_EXPIRED");
    }

    #[test]
    fn a_successful_resolve_renews_the_ttl() {
        let f = fixture();
        let (lease, refs) = bound(&f, "alice", "1", "snap-1");
        f.clock.fetch_add(TTL_MS - 1_000, Ordering::SeqCst);
        f.leases
            .resolve("alice", "1", &lease, "snap-1", &refs[0], 1)
            .unwrap();
        f.clock.fetch_add(TTL_MS - 1_000, Ordering::SeqCst);
        f.leases
            .resolve("alice", "1", &lease, "snap-1", &refs[0], 1)
            .unwrap();
    }

    #[test]
    fn resolve_after_suspend_is_focus_changed() {
        let f = fixture();
        let (lease, refs) = bound(&f, "alice", "1", "snap-1");
        f.leases.suspend(&lease);
        let err = f
            .leases
            .resolve("alice", "1", &lease, "snap-1", &refs[0], 1)
            .unwrap_err();
        assert_eq!(err.code, "DESKTOP_FOCUS_CHANGED");
    }

    #[test]
    fn resolve_with_an_old_epoch_is_ref_stale() {
        let f = fixture();
        let (lease, refs) = bound(&f, "alice", "1", "snap-1");
        let err = f
            .leases
            .resolve("alice", "1", &lease, "snap-1", &refs[0], 2)
            .unwrap_err();
        assert_eq!(err.code, "DESKTOP_REF_STALE");
    }

    #[test]
    fn resolve_checks_owner_window_snapshot_and_ref_in_order() {
        let f = fixture();
        let (lease, refs) = bound(&f, "alice", "1", "snap-1");
        let code = |r: Result<Resolved, DesktopError>| r.unwrap_err().code;
        assert_eq!(
            code(f.leases.resolve("bob", "1", &lease, "snap-1", &refs[0], 1)),
            "DESKTOP_LEASE_EXPIRED"
        );
        assert_eq!(
            code(
                f.leases
                    .resolve("alice", "other", &lease, "snap-1", &refs[0], 1)
            ),
            "DESKTOP_TARGET_CHANGED"
        );
        assert_eq!(
            code(
                f.leases
                    .resolve("alice", "1", &lease, "snap-0", &refs[0], 1)
            ),
            "DESKTOP_REF_STALE"
        );
        assert_eq!(
            code(
                f.leases
                    .resolve("alice", "1", &lease, "snap-1", "ref_bg_1_999", 1)
            ),
            "DESKTOP_REF_STALE"
        );
    }

    #[test]
    fn invalidate_snapshot_makes_refs_stale_but_keeps_the_native_snapshot() {
        let f = fixture();
        let (lease, refs) = bound(&f, "alice", "1", "snap-1");
        f.leases.invalidate_snapshot(&lease);
        let err = f
            .leases
            .resolve("alice", "1", &lease, "snap-1", &refs[0], 1)
            .unwrap_err();
        assert_eq!(err.code, "DESKTOP_REF_STALE");
        let released = f.leases.release("alice", "1", &lease).unwrap();
        assert_eq!(released.snapshot_id.as_deref(), Some("snap-1"));
    }

    #[test]
    fn release_returns_the_snapshot_id_not_the_lease_id() {
        // Review Focus 6: aevra frees the lease id by mistake.
        let f = fixture();
        let (lease, _) = bound(&f, "alice", "1", "snap-xyz");
        let released = f.leases.release("alice", "1", &lease).unwrap();
        assert_eq!(released.snapshot_id.as_deref(), Some("snap-xyz"));
        assert_ne!(released.snapshot_id.as_deref(), Some(lease.as_str()));
        assert!(f.leases.release("alice", "1", &lease).is_none());
    }

    #[test]
    fn release_requires_the_owner_and_the_window() {
        let f = fixture();
        let (lease, _) = bound(&f, "alice", "1", "snap-1");
        assert!(f.leases.release("bob", "1", &lease).is_none());
        assert!(f.leases.release("alice", "other", &lease).is_none());
        assert!(f.leases.release("alice", "1", &lease).is_some());
    }

    #[test]
    fn rebinding_reports_the_superseded_snapshot() {
        let f = fixture();
        let (lease, _) = bound(&f, "alice", "1", "snap-1");
        let b = f
            .leases
            .bind("alice", &lease, "snap-2", 1, vec![node("h9")])
            .unwrap();
        assert_eq!(b.superseded.as_deref(), Some("snap-1"));
        let released = f.leases.release("alice", "1", &lease).unwrap();
        assert_eq!(released.snapshot_id.as_deref(), Some("snap-2"));
    }

    #[test]
    fn release_for_app_releases_only_that_apps_leases() {
        let f = fixture();
        let a = f
            .leases
            .acquire("alice", &win("1"), 1, None, "app_a")
            .unwrap();
        f.leases
            .bind("alice", &a.lease_id, "snap-a", 1, vec![node("h")])
            .unwrap();
        let b = f
            .leases
            .acquire("alice", &win("2"), 1, None, "app_b")
            .unwrap();
        f.leases
            .bind("alice", &b.lease_id, "snap-b", 1, vec![node("h")])
            .unwrap();
        assert_eq!(
            f.leases.release_for_app("app_a"),
            vec!["snap-a".to_string()]
        );
        assert!(!f.leases.is_window_leased("1"));
        assert!(f.leases.is_window_leased("2"));
        assert_eq!(f.leases.lease_count_for_app("app_b"), 1);
    }

    #[test]
    fn expired_leases_hand_back_their_snapshots() {
        let f = fixture();
        let (_lease, _) = bound(&f, "alice", "1", "snap-1");
        f.clock.fetch_add(TTL_MS + 1, Ordering::SeqCst);
        assert_eq!(f.leases.take_pending_free(), vec!["snap-1".to_string()]);
        assert!(f.leases.take_pending_free().is_empty());
    }

    #[test]
    fn invalidate_owner_and_reset() {
        let f = fixture();
        bound(&f, "alice", "1", "snap-1");
        bound(&f, "bob", "2", "snap-2");
        assert_eq!(
            f.leases.invalidate_owner("alice"),
            vec!["snap-1".to_string()]
        );
        assert!(!f.leases.is_window_leased("1"));
        f.leases.reset();
        assert!(!f.leases.is_window_leased("2"));
    }

    #[test]
    fn sensitive_marks_travel_with_the_ref() {
        let f = fixture();
        let acq = f
            .leases
            .acquire("alice", &win("1"), 1, None, "app_a")
            .unwrap();
        let b = f
            .leases
            .bind(
                "alice",
                &acq.lease_id,
                "snap-1",
                1,
                vec![
                    node("h0"),
                    BoundNode {
                        handle: "h1".into(),
                        sensitive: Some("password"),
                    },
                ],
            )
            .unwrap();
        let r = f
            .leases
            .resolve("alice", "1", &acq.lease_id, "snap-1", &b.refs[1], 1)
            .unwrap();
        assert_eq!(r.sensitive, Some("password"));
    }

    #[test]
    fn expiry_is_rendered_as_rfc3339() {
        let f = fixture();
        let acq = f
            .leases
            .acquire("alice", &win("1"), 1, None, "app_a")
            .unwrap();
        assert!(acq.expires_at().ends_with('Z'));
    }
}
