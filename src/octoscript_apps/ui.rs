//! One native mini-app screen shared by standalone and embedded Rinx.
use super::{
    InstanceId, KernelProvider, Lease, OctosProvider, ServiceEvent,
    package::{Call, Package},
};
use makepad_widgets::splash_host::{splash_host_respond, take_splash_host_requests_for};
use makepad_widgets::*;
use octoscript_ui_l0::InstanceStore;
use octosense_app_policy::AssetServer;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub enum MiniAppsAction {
    Open,
    Close,
}
script_mod! {
    use mod.prelude.widgets.*
    mod.widgets.MiniAppsPanel = #(MiniAppsPanel::register_widget(vm)) {
        width: Fill height: Fill flow: Down padding: 18 spacing: 12
        show_bg: true
        draw_bg +: {color: instance(#xf5f5f5) pixel: fn() {return self.color}}
        header := View {width: Fill height: Fit flow: Right spacing: 12
            close := Button {text: "Back"}
            Label {text: "Mini apps" draw_text.color: #222 draw_text.text_style.font_size: 18}
        }
        import_form := View {width: Fill height: Fit flow: Down spacing: 8
            path := TextInput {width: Fill empty_text: "OctoSense bundle folder"}
            room := TextInput {width: Fill empty_text: "Room ID to allow (optional)"}
            core := View {width: Fill height: Fit flow: Down spacing: 6
                endpoint := TextInput {width: Fill empty_text: "Octos server URL"}
                profile := TextInput {width: Fill empty_text: "Octos profile"}
                token := TextInput {width: Fill is_password: true empty_text: "Octos access token"}
                connect := Button {text: "Connect Octos"}
            }
            buttons := View {width: Fill height: Fit spacing: 8
                review := Button {text: "Review bundle"}
                run := Button {text: "Run"}
            }
        }
        notice := Label {width: Fill height: Fit draw_text.color: #333 text: "Import an OctoSense bundle to review its services."}
        approval := View {visible: false width: Fill height: Fit flow: Down spacing: 8
            details := Label {width: Fill draw_text.color: #222}
            buttons := View {width: Fill height: Fit spacing: 8
                allow := Button {text: "Allow once"}
                deny := Button {text: "Deny"}
            }
        }
        card := Splash {width: Fill height: Fill}
    }
}
type Provider = Arc<dyn OctosProvider>;
struct Pending {
    reply: Option<(usize, u64)>,
    target: Option<String>,
    receiver: Receiver<ServiceEvent>,
    started: Instant,
    turn: bool,
}
struct Approval {
    id: String,
    message: String,
}
#[derive(Script, ScriptHook, Widget)]
pub struct MiniAppsPanel {
    #[deref]
    view: View,
    #[rust]
    open: bool,
    #[rust]
    reviewed_room: String,
    #[rust]
    package: Option<Package>,
    #[rust]
    lease: Option<Lease>,
    #[rust]
    connection_owner: Option<Provider>,
    #[rust]
    provider: Option<Provider>,
    #[rust]
    tag: String,
    #[rust]
    state: InstanceStore,
    #[rust]
    data: Value,
    #[rust]
    pending: Vec<Pending>,
    #[rust]
    approvals: VecDeque<Approval>,
    #[rust]
    assets: Option<AssetServer>,
}
impl Drop for MiniAppsPanel {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            lease.revoke();
            if let Some(provider) = self.provider.take() {
                provider.close(lease.identity());
            }
        }
    }
}
impl MiniAppsPanel {
    fn notice(&mut self, cx: &mut Cx, message: &str) {
        self.view.label(cx, ids!(notice)).set_text(cx, message);
    }
    fn stop(&mut self, cx: &mut Cx) {
        if let Some(lease) = self.lease.take() {
            lease.revoke();
            if let Some(provider) = self.provider.take() {
                provider.close(lease.identity());
            }
        }
        self.pending.clear();
        self.assets = None;
        self.approvals.clear();
        self.view.view(cx, ids!(approval)).set_visible(cx, false);
        self.view.splash(cx, ids!(card)).set_text(cx, "");
        self.view.view(cx, ids!(import_form)).set_visible(cx, true);
    }
    fn show_approval(&mut self, cx: &mut Cx) {
        if let Some(approval) = self.approvals.front() {
            self.view
                .label(cx, ids!(approval.details))
                .set_text(cx, &approval.message);
        }
        self.view
            .view(cx, ids!(approval))
            .set_visible(cx, !self.approvals.is_empty());
    }
    fn connect(&mut self, cx: &mut Cx) -> Result<(), String> {
        #[cfg(feature = "octosense-module")]
        if crate::module::is_hosted() {
            return Err("This app uses OctoSense's core connection".into());
        }
        let endpoint = self
            .view
            .text_input(cx, ids!(import_form.core.endpoint))
            .text();
        let url =
            url::Url::parse(endpoint.trim()).map_err(|_| "Enter a complete Octos server URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Use an HTTP or HTTPS URL without embedded credentials".into());
        }
        let profile = self
            .view
            .text_input(cx, ids!(import_form.core.profile))
            .text();
        if profile.trim().is_empty() {
            return Err("Enter the Octos profile name".into());
        }
        let token = self
            .view
            .text_input(cx, ids!(import_form.core.token))
            .text();
        let provider = KernelProvider::connect(octos_app_transport::TransportConfig {
            base_url: url,
            bearer: octos_app_transport::SecretString::new(token),
            profile_id: octos_app_transport::ProfileId::new(profile.trim()),
            cursor: None,
            cursor_file: None,
            requested_capabilities: octos_app_transport::Capabilities::requested(),
            workspace_cwd: None,
            stdio: None,
        })?;
        self.connection_owner = Some(provider);
        self.view
            .text_input(cx, ids!(import_form.core.token))
            .set_text(cx, "");
        self.notice(cx, "Octos connection configured. Review and run your app.");
        Ok(())
    }
    fn review(&mut self, cx: &mut Cx) -> Result<(), String> {
        self.stop(cx);
        self.package = None;
        let path = self.view.text_input(cx, ids!(path)).text();
        let snapshots = crate::app_data_dir().to_owned().join("miniapps/imports");
        let package = Package::load_in(&PathBuf::from(path.trim()), &snapshots)?;
        let room = self.view.text_input(cx, ids!(room)).text();
        if !room.trim().is_empty() {
            ruma::RoomId::parse(room.trim()).map_err(|_| "Invalid Matrix room ID")?;
        }
        let services = package.manifest.capabilities.join(", ");
        self.notice(cx,&format!("{} {} · Local unsigned bundle\nServices: {}\nAllowed room: {}\nRun grants these services for this session. Octos turns may use the connected core's tools.",package.manifest.name,package.manifest.version,services,if room.trim().is_empty(){"None"}else{room.trim()}));
        self.reviewed_room = room.trim().to_string();
        self.package = Some(package);
        Ok(())
    }
    fn run(&mut self, cx: &mut Cx) -> Result<(), String> {
        self.stop(cx);
        let package = self.package.as_ref().ok_or("Review a bundle first")?;
        package.unchanged()?;
        let account = crate::sliding_sync::current_user_id()
            .ok_or("Log in to Matrix before running a mini app")?
            .to_string();
        let room = self.view.text_input(cx, ids!(room)).text();
        if room.trim() != self.reviewed_room {
            return Err("Room access changed. Review the bundle again".into());
        }
        let room = if room.trim().is_empty() {
            None
        } else {
            Some(
                ruma::RoomId::parse(room.trim())
                    .map_err(|_| "Invalid room ID")?
                    .to_string(),
            )
        };
        static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let lease = super::AUTHORITY.issue(
            InstanceId {
                app: package.manifest.id.clone(),
                account: account.clone(),
                room: room.clone(),
                generation,
            },
            package.manifest.capabilities.iter().cloned().collect(),
            room.into_iter().collect(),
            Instant::now() + Duration::from_secs(3600),
        );
        self.tag = format!("rinx-miniapp-{generation}");
        self.data = package.data.clone();
        self.state = Default::default();
        if !package.script {
            for (field, value) in octoscript_ui_l0::state_initials(&package.source) {
                self.state
                    .set_cell(octoscript_ui_l0::CARD_STATE_KEY, &field, value);
            }
        }
        let server = octosense_app_policy::AssetServer::start(&package.root)?;
        octosense_app_policy::rewrite_assets(&mut self.data, server.origin());
        let account_dir: String = account.bytes().map(|b| format!("{b:02x}")).collect();
        // The local core's default read boundary is its data root. Allocate
        // an account/app child there and then narrow the session to that child.
        // The location comes from the native host, never from bundle input.
        let root = octos_app_transport::shared::Connection::current()
            .and_then(|c| c.local_data_root.clone())
            .unwrap_or_else(|| crate::app_data_dir().to_owned())
            .join("miniapps")
            .join(account_dir);
        let mut settings = package.policy.isolate_settings(&root);
        std::fs::create_dir_all(&settings.jail_root).map_err(|e| e.to_string())?;
        self.provider =
            KernelProvider::shared(&settings.jail_root)?.map(|p| p as Arc<dyn OctosProvider>);
        settings.hosts.push(server.allowlist_entry());
        settings.allow_net = true;
        if !package.script {
            settings.capabilities.push("rinx.event".into());
        }
        if !settings.capabilities.iter().any(|s| s == "net") {
            settings.capabilities.push("net".into());
        }
        let splash = self.view.splash(cx, ids!(card));
        octosense_app_policy::splash_adapter::apply(&splash, cx, &settings);
        splash.set_host_tag(cx, Some(self.tag.clone()));
        self.assets = Some(server);
        self.lease = Some(lease);
        self.render(cx)?;
        let calls = self.package.as_ref().unwrap().bindings.on_open.clone();
        for call in calls {
            self.binding(cx, call, "root", &Value::Null)?;
        }
        self.view.view(cx, ids!(import_form)).set_visible(cx, false);
        self.notice(
            cx,
            "Running · Back closes this app and revokes its services.",
        );
        Ok(())
    }
    fn render(&mut self, cx: &mut Cx) -> Result<(), String> {
        let package = self.package.as_ref().ok_or("No package")?;
        if package.script {
            let source = octosense_app_policy::script_source(
                &package.root,
                self.assets
                    .as_ref()
                    .ok_or("Missing bundle assets")?
                    .origin(),
            )
            .ok_or("Missing main.splash")??;
            self.view.splash(cx, ids!(card)).set_text(cx, &source);
            return Ok(());
        }
        let report = octoscript_ui_l0::realize_with_state(
            &package.source,
            &self.data,
            &self.state,
            Default::default(),
        );
        report.complete_root()?;
        self.state.prune(&report.live_keys);
        for (field, value) in report.captured {
            self.state
                .set_cell(octoscript_ui_l0::CARD_STATE_KEY, &field, value);
        }
        let mut card = octoscript_makepad::l0::prepare_with_state(
            &package.source,
            &self.data,
            &self.state,
            &package.root.join("kit"),
        )?;
        octoscript_makepad::l0::inspectable(&mut card.tree);
        let ui = if card.native_components {
            octoscript_makepad::design::to_makepad_ui(&card.tree)?
        } else {
            octoscript_makepad::to_makepad_l0_ui(&card.tree)
        };
        // NAV belongs to this isolate and enters the same authenticated queue
        // as imperative host.request calls. No global Notify event is trusted.
        let body = format!(
            "let NAV = fn(t, v=\"\") {{ host.request(\"rinx.event\", {{route:t, value:v}}, fn(r){{}}) }}\nwidth:Fill height:Fill flow:Down\n{ui}"
        );
        self.view.splash(cx, ids!(card)).reapply_text(cx, &body);
        Ok(())
    }
    fn dispatch(
        &mut self,
        service: &str,
        args: Value,
        reply: Option<(usize, u64)>,
        target: Option<String>,
    ) -> Result<(), String> {
        if target.is_some() && self.pending.iter().any(|p| p.target == target) {
            return Err("This action is already running".into());
        }
        if self.pending.len() >= 16 {
            return Err("Mini app already has 16 pending requests".into());
        }
        let lease = self.lease.clone().ok_or("Mini app is closed")?;
        let account = crate::sliding_sync::current_user_id().ok_or("Not logged in")?;
        lease.authorize(account.as_str(), service, None)?;
        rinx_miniapp_core::parse_arguments(&args.to_string())?;
        let (tx, rx) = mpsc::sync_channel(128);
        if service.starts_with("matrix.") {
            let service = service.to_owned();
            crate::sliding_sync::spawn_async_task(async move {
                let result = super::matrix_request(lease, service, args).await;
                let _ = tx.try_send(ServiceEvent::Complete(result));
                SignalToUI::set_ui_signal();
            });
        } else if service.starts_with("octos.") {
            self.provider.as_ref().ok_or("Octos unavailable. Open AppCard in OctoSense to connect the core, then reopen this mini app.")?.request(lease,service,args,tx)?;
        } else {
            return Err(format!("No adapter for {service}"));
        }
        self.pending.push(Pending {
            reply,
            target,
            receiver: rx,
            started: Instant::now(),
            turn: service == "octos.turn.start",
        });
        Ok(())
    }
    fn binding(
        &mut self,
        _cx: &mut Cx,
        call: Call,
        key: &str,
        payload: &Value,
    ) -> Result<(), String> {
        let args = super::package::arguments(&call.args, &self.data, &self.state, key, payload)?;
        self.dispatch(&call.service, args, None, Some(call.target))
    }
    fn event(&mut self, cx: &mut Cx, args: Value) -> Result<(), String> {
        let route = args["route"].as_str().ok_or("Missing event route")?;
        let mut event: Value = serde_json::from_str(
            route
                .strip_prefix("l0:")
                .ok_or("Unknown navigation event")?,
        )
        .map_err(|e| e.to_string())?;
        if event["v"] == "$$" {
            event["v"] = args["value"].clone();
        }
        let key = event["k"].as_str().ok_or("Missing instance key")?;
        let name = event["e"].as_str().ok_or("Missing event name")?;
        let package = self.package.as_ref().ok_or("No package")?;
        if package.script {
            return Err("L0 events are not available to main.splash apps".into());
        }
        let changed = octoscript_ui_l0::dispatch_with_data(
            &package.source,
            &mut self.state,
            key,
            name,
            Some(&event["v"]),
            &self.data,
        );
        let call = package.bindings.events.get(name).cloned();
        if changed {
            self.render(cx)?;
        }
        if let Some(call) = call {
            self.binding(cx, call, key, &event["v"])?;
        }
        Ok(())
    }
    fn pump(&mut self, cx: &mut Cx) {
        let Some(lease) = self.lease.clone() else {
            return;
        };
        if lease
            .check(
                &crate::sliding_sync::current_user_id()
                    .map(|u| u.to_string())
                    .unwrap_or_default(),
            )
            .is_err()
        {
            self.stop(cx);
            self.notice(cx, "Session ended. Review and run again.");
            return;
        }
        let heap = self.view.splash(cx, ids!(card)).isolate_heap_key(cx);
        let owned_heaps: Vec<_> = heap.into_iter().collect();
        for req in take_splash_host_requests_for(&owned_heaps) {
            let result = rinx_miniapp_core::parse_arguments(&req.args_json).and_then(|args| {
                if req.service == "rinx.event" {
                    self.event(cx, args)
                } else {
                    self.dispatch(&req.service, args, Some((req.heap_key, req.req_id)), None)
                }
            });
            if req.service == "rinx.event" || result.is_err() {
                splash_host_respond(
                    cx,
                    req.heap_key,
                    req.req_id,
                    result.as_ref().map(|_| "{}").map_err(String::as_str),
                );
                if let Err(e) = result {
                    self.notice(cx, &e);
                }
            }
        }
        let mut updates = Vec::new();
        self.pending.retain_mut(|pending| {
            let mut done = false;
            loop {
                match pending.receiver.try_recv() {
                    Ok(event) => {
                        let complete = matches!(event, ServiceEvent::Complete(_));
                        updates.push((pending.reply, pending.target.clone(), pending.turn, event));
                        if complete {
                            done = true;
                            break;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        updates.push((
                            pending.reply,
                            pending.target.clone(),
                            pending.turn,
                            ServiceEvent::Complete(Err(
                                "Service connection closed before completing".into(),
                            )),
                        ));
                        done = true;
                        break;
                    }
                }
            }
            if !done && pending.started.elapsed() > Duration::from_secs(185) {
                updates.push((
                    pending.reply,
                    pending.target.clone(),
                    pending.turn,
                    ServiceEvent::Complete(Err("Service timed out".into())),
                ));
                done = true;
            }
            !done
        });
        let mut redraw = false;
        for (reply, target, turn, event) in updates {
            let (complete, result) = match event {
                ServiceEvent::Data(v) => (false, Ok(v)),
                ServiceEvent::Complete(r) => (true, r),
            };
            if !complete {
                if let Ok(value) = &result {
                    let event = &value["event"];
                    if event["kind"] == "approval_requested" {
                        if let Some(id) = event["approval_id"].as_str() {
                            if !self.approvals.iter().any(|a| a.id == id) {
                                self.approvals.push_back(Approval {
                                    id: id.to_owned(),
                                    message: format!(
                                        "Octos tool approval: {}\n{}",
                                        event["title"].as_str().unwrap_or("Tool request"),
                                        event["body"].as_str().unwrap_or("")
                                    ),
                                });
                            }
                            self.show_approval(cx);
                        }
                    }
                }
            }
            if let Some(target) = target {
                let value = match &result {
                    Ok(v) => json!({"is_ok":true,"data":v}),
                    Err(e) => json!({"is_ok":false,"error":e}),
                };
                self.data[&target] = value;
                redraw = true;
            }
            if complete {
                if turn {
                    self.approvals.clear();
                    self.show_approval(cx);
                }
                if let Some((heap, req)) = reply {
                    let text = result.as_ref().map(|v| v.to_string());
                    splash_host_respond(cx, heap, req, text.as_deref().map_err(|e| e.as_str()));
                }
                if let Err(error) = result {
                    self.notice(cx, &error);
                }
            }
        }
        if redraw {
            if let Err(e) = self.render(cx) {
                self.notice(cx, &e);
            }
        }
    }
}
impl Widget for MiniAppsPanel {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        if !self.open {
            return;
        }
        self.view.handle_event(cx, event, scope);
        if let Event::Actions(actions) = event {
            if self.view.button(cx, ids!(close)).clicked(actions) {
                cx.action(MiniAppsAction::Close);
            }
            let approve = self
                .view
                .button(cx, ids!(approval.buttons.allow))
                .clicked(actions);
            let deny = self
                .view
                .button(cx, ids!(approval.buttons.deny))
                .clicked(actions);
            if approve || deny {
                if let (Some(approval), Some(provider), Some(lease)) = (
                    self.approvals.pop_front(),
                    self.provider.clone(),
                    self.lease.clone(),
                ) {
                    let (tx, rx) = mpsc::sync_channel(1);
                    match provider.decide(lease, &approval.id, approve, tx) {
                        Ok(()) => self.pending.push(Pending {
                            reply: None,
                            target: None,
                            receiver: rx,
                            started: Instant::now(),
                            turn: false,
                        }),
                        Err(e) => self.notice(cx, &e),
                    }
                    self.show_approval(cx);
                }
            }
            let result = if self
                .view
                .button(cx, ids!(import_form.core.connect))
                .clicked(actions)
            {
                self.connect(cx)
            } else if self.view.button(cx, ids!(review)).clicked(actions) {
                self.review(cx)
            } else if self.view.button(cx, ids!(run)).clicked(actions) {
                self.run(cx)
            } else {
                Ok(())
            };
            if let Err(e) = result {
                self.stop(cx);
                self.notice(cx, &e);
            }
        }
        if let Event::BackPressed { handled } = event {
            handled.set(true);
            cx.action(MiniAppsAction::Close);
        }
        self.pump(cx);
    }
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        self.view.draw_walk(cx, scope, walk)
    }
}
impl MiniAppsPanelRef {
    pub fn action(&self, cx: &mut Cx, modal: ModalRef, action: &MiniAppsAction) {
        if let Some(mut inner) = self.borrow_mut() {
            match action {
                MiniAppsAction::Open => {
                    inner.open = true;
                    inner.notice(cx, "Import an OctoSense bundle to review its services.");
                    #[cfg(feature = "octosense-module")]
                    inner
                        .view
                        .view(cx, ids!(import_form.core))
                        .set_visible(cx, !crate::module::is_hosted());
                    modal.open(cx);
                }
                MiniAppsAction::Close => {
                    inner.stop(cx);
                    inner.open = false;
                    modal.close(cx);
                }
            }
        }
    }
}
