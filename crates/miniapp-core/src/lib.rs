//! Portable contracts shared by standalone and embedded mini-app hosts.
pub mod matrix;

use serde_json::Value;
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

pub const MAX_REQUEST_BYTES: usize = 256 * 1024;
pub const MAX_REPLY_BYTES: usize = 2 * 1024 * 1024;

/// Identity is assigned by the embedding host, never deserialized from script arguments.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InstanceId {
    pub app: String,
    pub account: String,
    pub room: Option<String>,
    pub generation: u64,
}

/// Revocation is shared with asynchronous work, so closing an instance also
/// invalidates already queued requests and replies.
#[derive(Clone, Debug)]
pub struct Lease {
    identity: InstanceId,
    services: BTreeSet<String>,
    rooms: BTreeSet<String>,
    expires: Instant,
    alive: Arc<AtomicBool>,
    authority: Arc<AtomicU64>,
    epoch: u64,
}

#[derive(Clone, Debug, Default)]
pub struct SessionAuthority(Arc<AtomicU64>);
impl SessionAuthority {
    pub fn invalidate(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
    pub fn issue(
        &self,
        identity: InstanceId,
        services: BTreeSet<String>,
        rooms: BTreeSet<String>,
        expires: Instant,
    ) -> Lease {
        Lease {
            identity,
            services,
            rooms,
            expires,
            alive: Arc::new(AtomicBool::new(true)),
            authority: self.0.clone(),
            epoch: self.0.load(Ordering::Acquire),
        }
    }
}

impl Lease {
    /// Called only after the host has intersected requested and approved grants.
    pub fn new(
        identity: InstanceId,
        services: BTreeSet<String>,
        rooms: BTreeSet<String>,
        expires: Instant,
    ) -> Self {
        SessionAuthority::default().issue(identity, services, rooms, expires)
    }
    pub fn identity(&self) -> &InstanceId {
        &self.identity
    }
    pub fn services(&self) -> &BTreeSet<String> {
        &self.services
    }
    pub fn revoke(&self) {
        self.alive.store(false, Ordering::Release);
    }
    pub fn check(&self, account: &str) -> Result<(), String> {
        if !self.alive.load(Ordering::Acquire)
            || Instant::now() >= self.expires
            || self.authority.load(Ordering::Acquire) != self.epoch
        {
            return Err("Mini-app authorization expired".into());
        }
        if account != self.identity.account {
            return Err("Mini-app account changed".into());
        }
        Ok(())
    }
    pub fn authorize(
        &self,
        account: &str,
        service: &str,
        room: Option<&str>,
    ) -> Result<(), String> {
        self.check(account)?;
        if !self.services.contains(service) {
            return Err(format!("Mini app was not granted {service}"));
        }
        if let Some(room) = room {
            if !self.rooms.contains(room) {
                return Err("Mini app was not granted access to this room".into());
            }
        }
        Ok(())
    }
    pub fn permits_room(&self, room: &str) -> bool {
        self.rooms.contains(room)
    }
}

pub fn parse_arguments(json: &str) -> Result<Value, String> {
    if json.len() > MAX_REQUEST_BYTES {
        return Err("Mini-app request is too large".into());
    }
    let value: Value =
        serde_json::from_str(json).map_err(|e| format!("Invalid service arguments: {e}"))?;
    if !value.is_object() {
        return Err("Service arguments must be an object".into());
    }
    Ok(value)
}

pub fn bounded_reply(value: Value) -> Result<Value, String> {
    if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > MAX_REPLY_BYTES {
        return Err("Mini-app response is too large".into());
    }
    Ok(value)
}

/// One asynchronous response or stream item. A provider must finish a request
/// exactly once; dropping its receiver cancels delivery to a departed app.
#[derive(Debug)]
pub enum ServiceEvent {
    Data(Value),
    Complete(Result<Value, String>),
}

/// The Octos connection is supplied by the host. It carries no Matrix tokens
/// and does not own or spawn a kernel merely because a mini app is opened.
pub trait OctosProvider: Send + Sync {
    fn request(
        &self,
        lease: Lease,
        service: &str,
        args: Value,
        reply: std::sync::mpsc::SyncSender<ServiceEvent>,
    ) -> Result<(), String>;
    /// Called by native host approval UI, never by script service arguments.
    fn decide(
        &self,
        lease: Lease,
        approval: &str,
        approve: bool,
        reply: std::sync::mpsc::SyncSender<ServiceEvent>,
    ) -> Result<(), String>;
    fn close(&self, identity: &InstanceId);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn lease() -> Lease {
        Lease::new(
            InstanceId {
                app: "test".into(),
                account: "@alice:example.org".into(),
                room: Some("!allowed:example.org".into()),
                generation: 7,
            },
            ["matrix.read_messages".into()].into(),
            ["!allowed:example.org".into()].into(),
            Instant::now() + Duration::from_secs(60),
        )
    }
    #[test]
    fn queued_work_observes_revocation_and_account_changes() {
        let original = lease();
        let queued = original.clone();
        assert!(queued.check("@bob:example.org").is_err());
        assert!(queued.check("@alice:example.org").is_ok());
        original.revoke();
        assert!(queued.check("@alice:example.org").is_err());
    }
    #[test]
    fn read_scope_cannot_be_used_to_publish_or_read_another_room() {
        let lease = lease();
        let account = "@alice:example.org";
        assert!(
            lease
                .authorize(
                    account,
                    "matrix.read_messages",
                    Some("!allowed:example.org")
                )
                .is_ok()
        );
        assert!(
            lease
                .authorize(account, "matrix.send_message", Some("!allowed:example.org"))
                .is_err()
        );
        assert!(
            lease
                .authorize(account, "matrix.read_messages", Some("!other:example.org"))
                .is_err()
        );
        assert!(lease.authorize(account, "octos.turn.start", None).is_err());
    }
    #[test]
    fn script_arguments_cannot_supply_host_identity() {
        assert!(parse_arguments("[]").is_err());
        let lease = lease();
        let _forged = parse_arguments(
            r#"{"account":"@alice:example.org","room":"!allowed:example.org","generation":7}"#,
        )
        .unwrap();
        assert!(lease.check("@bob:example.org").is_err());
    }
}
