//! Octos adapter over the SAME connection AppCard publishes on its UI thread.
use octos_app_transport::shared::Connection;
use rinx_miniapp_core::{InstanceId, Lease, OctosProvider, ServiceEvent};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc::SyncSender,
};

pub struct KernelProvider {
    connection: Arc<Connection>,
    session: String,
    active_turn: Arc<AtomicU64>,
    turn_namespace: u64,
    workspace: Option<String>,
}
impl KernelProvider {
    pub fn shared(workspace: &std::path::Path) -> Result<Option<Arc<Self>>, String> {
        let Some(connection) = Connection::current() else {
            return Ok(None);
        };
        let workspace = workspace.canonicalize().map_err(|e| e.to_string())?;
        Ok(Some(Self::from_connection(
            connection,
            Some(workspace.to_string_lossy().into_owned()),
        )))
    }
    fn from_connection(connection: Arc<Connection>, workspace: Option<String>) -> Arc<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let session = format!("{}:api:rinx-mini-{nonce:x}-{id}", connection.profile);
        Arc::new(Self {
            connection,
            session,
            active_turn: Arc::new(AtomicU64::new(0)),
            turn_namespace: (uuid::Uuid::new_v4().as_u128() >> 64) as u64,
            workspace,
        })
    }
    /// Explicit standalone configuration uses the identical WS/stdio protocol.
    pub fn connect(config: octos_app_transport::TransportConfig) -> Result<Arc<Self>, String> {
        let (connection, receiver) =
            Connection::start(config, Arc::new(makepad_widgets::SignalToUI::set_ui_signal))?;
        drop(receiver);
        Ok(Self::from_connection(connection, None))
    }
}
fn turn_id(namespace: u64, number: u64) -> String {
    uuid::Uuid::from_u128(((namespace as u128) << 64) | number as u128).to_string()
}
fn valid(lease: &Lease) -> Result<(), String> {
    let account = crate::sliding_sync::current_user_id().ok_or("Not logged in")?;
    lease.check(account.as_str())
}
impl OctosProvider for KernelProvider {
    fn request(
        &self,
        lease: Lease,
        service: &str,
        args: Value,
        reply: SyncSender<ServiceEvent>,
    ) -> Result<(), String> {
        valid(&lease)?;
        let workspace = self
            .workspace
            .clone()
            .ok_or("Open the mini app to bind its Octos workspace")?;
        if !self.connection.is_active() {
            return Err("Octos connection changed; reopen this app".into());
        }
        lease.authorize(&lease.identity().account, service, None)?;
        let method = match service {
            "octos.session.open" => "session/open",
            "octos.session.history" => "session/hydrate",
            "octos.turn.start" => "turn/start",
            "octos.turn.interrupt" => "turn/interrupt",
            _ => return Err("Unknown Octos mini-app service".into()),
        };
        // Apps supply input text, never session/profile/workspace identity,
        // approval decisions, arbitrary RPC methods or filesystem paths.
        let allowed: &[&str] = if method == "turn/start" {
            &["text"]
        } else {
            &[]
        };
        if args
            .as_object()
            .is_none_or(|o| o.keys().any(|k| !allowed.contains(&k.as_str())))
        {
            return Err("Unsupported Octos arguments".into());
        }
        let session = self.session.clone();
        static NEXT_TURN: AtomicU64 = AtomicU64::new(1);
        let turn_number = if method == "turn/start" {
            NEXT_TURN.fetch_add(1, Ordering::Relaxed)
        } else {
            self.active_turn.load(Ordering::Acquire)
        };
        if method == "turn/interrupt" && turn_number == 0 {
            return Err("No active Octos turn".into());
        }
        let turn = turn_id(self.turn_namespace, turn_number);
        let params = match method {
            "session/open" => json!({"session_id":session,"profile_id":self.connection.profile,
                "cwd":workspace,"sandbox":{"enabled":true,"network_access":false,"read_allow_paths":[workspace]}}),
            "session/hydrate" => json!({"session_id":session,"include":["messages"]}),
            "turn/start" => {
                let text = args["text"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty() && s.len() <= 32 * 1024)
                    .ok_or("Provide text (at most 32 KiB)")?;
                json!({"session_id":session,"turn_id":turn,"input":[{"kind":"text","text":text}]})
            }
            _ => json!({"session_id":session,"turn_id":turn}),
        };
        if method == "turn/start"
            && self
                .active_turn
                .compare_exchange(0, turn_number, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return Err("This app already has an active Octos turn".into());
        }
        let active_turn = self.active_turn.clone();
        let connection = self.connection.clone();
        self.connection.handle().spawn(async move {
            let mut events = connection.subscribe();
            let work = async {
                valid(&lease)?;
                if matches!(method,"turn/start"|"session/hydrate") {
                    connection.request("session/open",json!({"session_id":session,"profile_id":connection.profile,
                        "cwd":workspace,"sandbox":{"enabled":true,"network_access":false,"read_allow_paths":[workspace]}})).await?;
                }
                let result = connection.request(method, params).await?;
                valid(&lease)?;
                if method != "turn/start" { return Ok(result); }
                let mut stream = TurnReply::default();
                loop {
                    let event = events.recv().await.map_err(|_| "Octos event stream closed or fell behind; reload history")?;
                    if event.session_id().0 != session { continue; }
                    let data = serde_json::to_value(&*event).map_err(|e| e.to_string())?;
                    valid(&lease)?;
                    if !stream.accept(&turn, &data)? {continue;}
                    if stream.completed {
                        // The terminal can overtake the persisted transcript lane.
                        // Hydrate by THIS turn, never substitute an older answer.
                        let history=connection.request("session/hydrate",json!({"session_id":session,"include":["messages"]})).await?;
                        valid(&lease)?;
                        if let Some(text)=history["messages"].as_array().and_then(|rows|rows.iter().rev().find(|m|m["role"]=="assistant" && m["turn_id"]==turn)).and_then(|m|m["content"].as_str()) {
                            stream.text=text.to_owned();
                        }
                        return rinx_miniapp_core::bounded_reply(json!({"text":stream.text,"event":data}));
                    }
                    let data=rinx_miniapp_core::bounded_reply(json!({"text":stream.text,"event":data}))?;
                    reply.try_send(ServiceEvent::Data(data)).map_err(|_| "Mini-app event consumer is unavailable")?;
                    makepad_widgets::SignalToUI::set_ui_signal();
                }
            };
            tokio::pin!(work);
            let timeout = tokio::time::sleep(std::time::Duration::from_secs(180));
            tokio::pin!(timeout);
            let result = loop {
                tokio::select! {
                    r = &mut work => break r,
                    _ = &mut timeout => break Err("Octos turn timed out".into()),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                        if let Err(e) = valid(&lease) { break Err(e); }
                        if !connection.is_active() { break Err("Octos connection changed".into()); }
                    }
                }
            };
            if result.is_err() && method == "turn/start" {
                let _ = connection.request("turn/interrupt", json!({"session_id":session,"turn_id":turn})).await;
            }
            if method=="turn/start" { let _=active_turn.compare_exchange(turn_number,0,Ordering::AcqRel,Ordering::Acquire); }
            let result = result.map_err(|error| {
                if error.contains("401") && error.to_ascii_lowercase().contains("authentication") {
                    "Octos provider authentication failed (HTTP 401). Update the provider credentials in AppCard.".into()
                } else {
                    error
                }
            });
            let _ = reply.try_send(ServiceEvent::Complete(result));
            makepad_widgets::SignalToUI::set_ui_signal();
        });
        Ok(())
    }
    fn decide(
        &self,
        lease: Lease,
        approval: &str,
        approve: bool,
        reply: SyncSender<ServiceEvent>,
    ) -> Result<(), String> {
        valid(&lease)?;
        let connection = self.connection.clone();
        let params = json!({"session_id":self.session,"approval_id":approval,"decision":if approve{"approve"}else{"deny"}});
        self.connection.handle().spawn(async move {
            let result = async {
                valid(&lease)?;
                let result = connection.request("approval/respond", params).await?;
                valid(&lease)?;
                Ok(result)
            }
            .await;
            let _ = reply.try_send(ServiceEvent::Complete(result));
            makepad_widgets::SignalToUI::set_ui_signal();
        });
        Ok(())
    }
    fn close(&self, _identity: &InstanceId) {
        let number = self.active_turn.load(Ordering::Acquire);
        if number == 0 {
            return;
        }
        let connection = self.connection.clone();
        let session = self.session.clone();
        let turn = turn_id(self.turn_namespace, number);
        self.connection.handle().spawn(async move {
            let _ = connection
                .request(
                    "turn/interrupt",
                    json!({"session_id":session,"turn_id":turn}),
                )
                .await;
        });
    }
}

/// Consume both protocol generations while keeping turn and segment ownership.
#[derive(Default)]
struct TurnReply {
    text: String,
    segment: String,
    sequence: std::collections::BTreeMap<String, u64>,
    v2: bool,
    completed: bool,
}
impl TurnReply {
    fn accept(&mut self, turn: &str, event: &Value) -> Result<bool, String> {
        let envelope = &event["envelope"];
        let event_turn = event
            .get("turn_id")
            .or_else(|| envelope.get("turn_id"))
            .and_then(Value::as_str);
        if event_turn.is_some_and(|id| id != turn) {
            return Ok(false);
        }
        match event["kind"].as_str().unwrap_or("") {
            "envelope_v2" => {
                if event_turn != Some(turn) {
                    return Ok(false);
                }
                if let (Some(thread), Some(seq)) =
                    (envelope["thread_id"].as_str(), envelope["seq"].as_u64())
                {
                    if self.sequence.get(thread).is_some_and(|seen| seq <= *seen) {
                        return Ok(false);
                    }
                    self.sequence.insert(thread.to_owned(), seq);
                }
                let payload = &envelope["payload"];
                let data = &payload["data"];
                match payload["type"].as_str().unwrap_or("") {
                    "assistant_delta" | "assistant_persisted" => {
                        let segment = data["assistant_segment_id"].as_str().unwrap_or("");
                        if !self.v2 || self.segment != segment {
                            self.text.clear();
                            self.segment = segment.into();
                        }
                        self.v2 = true;
                        let text = data["text"].as_str().unwrap_or("");
                        if payload["type"] == "assistant_persisted" {
                            self.text = text.into();
                        } else {
                            self.text.push_str(text);
                        }
                    }
                    "turn_terminal" => {
                        if data["outcome"] != "completed" {
                            return Err(data["error"]["message"]
                                .as_str()
                                .unwrap_or("Octos turn ended without completing")
                                .to_owned());
                        }
                        self.completed = true;
                    }
                    _ => {}
                }
            }
            "message_delta" if !self.v2 => self.text.push_str(event["text"].as_str().unwrap_or("")),
            "turn_completed" => self.completed = true,
            "turn_error" => {
                return Err(event["message"]
                    .as_str()
                    .unwrap_or("Octos turn failed")
                    .into());
            }
            _ => {}
        }
        Ok(true)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v2_final_replaces_deltas_and_ignores_other_turns_and_replay() {
        let mut reply = TurnReply::default();
        assert!(
            !reply
                .accept(
                    "owned",
                    &json!({"kind":"message_delta","turn_id":"other","text":"private"})
                )
                .unwrap()
        );
        reply
            .accept(
                "owned",
                &json!({"kind":"message_delta","turn_id":"owned","text":"draft"}),
            )
            .unwrap();
        let event = json!({"kind":"envelope_v2","envelope":{"turn_id":"owned","thread_id":"thread","seq":1,"payload":{"type":"assistant_persisted","data":{"text":"final answer","assistant_segment_id":"segment"}}}});
        assert!(reply.accept("owned", &event).unwrap());
        assert!(!reply.accept("owned", &event).unwrap());
        assert_eq!(reply.text, "final answer");
        reply
            .accept(
                "owned",
                &json!({"kind":"message_delta","turn_id":"owned","text":"duplicate"}),
            )
            .unwrap();
        assert_eq!(reply.text, "final answer");
        assert!(
            reply
                .accept(
                    "owned",
                    &json!({"kind":"turn_error","turn_id":"owned","message":"denied"})
                )
                .is_err()
        );
    }
    #[test]
    fn host_turn_ids_are_protocol_uuids() {
        assert!(uuid::Uuid::parse_str(&turn_id(123, 1)).is_ok());
        assert_ne!(turn_id(123, 1), turn_id(123, 2));
        assert_ne!(turn_id(123, 1), turn_id(124, 1));
    }
}
