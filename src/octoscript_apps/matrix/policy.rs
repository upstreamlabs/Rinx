//! Request-scoped SDK identity and explicit room grants. No global SDK lookup
//! can redirect queued work into a replacement login session.
use matrix_sdk::{Client, ruma::OwnedUserId};
use rinx_miniapp_core::Lease;
use std::future::IntoFuture;

pub const ROOM_ACCESS_DENIED: &str = "Mini app was not granted access to this room";
#[derive(Clone, Copy)]
pub enum RoomAccess {
    Read,
    Write,
}
struct Context {
    client: Client,
    lease: Lease,
    service: String,
}
tokio::task_local! { static CONTEXT: Context; }

pub async fn with_context<T>(
    client: Client,
    lease: Lease,
    service: String,
    future: impl std::future::Future<Output = T>,
) -> T {
    CONTEXT
        .scope(
            Context {
                client,
                lease,
                service,
            },
            future,
        )
        .await
}
pub fn check() -> Result<(), String> {
    if crate::logout::logout_state_machine::is_logout_in_progress() {
        return Err("Matrix session is closing".into());
    }
    let account = crate::sliding_sync::current_user_id().ok_or("Not logged in")?;
    CONTEXT
        .try_with(|ctx| ctx.lease.check(account.as_str()))
        .map_err(|_| "Missing mini-app request context")?
}
pub fn get_client() -> Option<Client> {
    check().ok()?;
    CONTEXT.try_with(|ctx| ctx.client.clone()).ok()
}
pub fn current_user_id() -> Option<OwnedUserId> {
    get_client()?.user_id().map(ToOwned::to_owned)
}

fn writing(service: &str) -> bool {
    matches!(
        service,
        "matrix.send_message"
            | "matrix.rooms_send"
            | "matrix.reply"
            | "matrix.thread_reply"
            | "matrix.react"
            | "matrix.typing"
            | "matrix.read_receipt"
            | "matrix.pin"
            | "matrix.favorite"
            | "matrix.low_priority"
            | "matrix.mark_unread"
            | "matrix.invite"
            | "matrix.join"
            | "matrix.invite_respond"
            | "matrix.dm_open"
    )
}
pub fn room_access_allowed(room: &str, access: RoomAccess) -> bool {
    ensure_room_access(room, access).is_ok()
}
pub fn ensure_room_access(room: &str, access: RoomAccess) -> Result<(), String> {
    check()?;
    CONTEXT.with(|ctx| {
        if !ctx.lease.permits_room(room)
            || (matches!(access, RoomAccess::Write) && !writing(&ctx.service))
        {
            return Err(ROOM_ACCESS_DENIED.into());
        }
        Ok(())
    })
}
pub fn global_room_access_allowed(room: &str, access: RoomAccess) -> bool {
    if room.is_empty() && matches!(access, RoomAccess::Write) {
        return check().is_ok() && CONTEXT.with(|ctx| ctx.service == "matrix.dm_open");
    }
    room_access_allowed(room, access)
}
pub fn ensure_server_output(server: &str) -> Result<(), String> {
    check()?;
    CONTEXT.with(|ctx| {
        if ctx.client.homeserver().as_str() != server {
            return Err("Matrix endpoint changed".into());
        }
        Ok(())
    })
}
pub fn commit_sensitive_target(target: &str, _content: &serde_json::Value) -> Result<(), String> {
    check()?;
    if target == "host" && CONTEXT.with(|ctx| ctx.service == "matrix.dm_open") {
        return Ok(());
    }
    ensure_room_access(target, RoomAccess::Write)
}
pub async fn audit_server_operation<F, T, E>(server: &str, future: F) -> Result<T, String>
where
    F: IntoFuture<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    ensure_server_output(server)?;
    let future = future.into_future();
    tokio::pin!(future);
    // Stop waiting on a revoked account/instance, including when the server stalls.
    let result = loop {
        tokio::select! {
            result = &mut future => break result.map_err(|e| e.to_string()),
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => check()?,
        }
    };
    check()?;
    result
}
pub async fn audit_room_operation<F, T, E>(room: &str, future: F) -> Result<T, String>
where
    F: IntoFuture<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    ensure_room_access(room, RoomAccess::Write)?;
    let server = get_client()
        .ok_or("Not logged in")?
        .homeserver()
        .to_string();
    let result = audit_server_operation(&server, future).await;
    ensure_room_access(room, RoomAccess::Write)?;
    result
}
