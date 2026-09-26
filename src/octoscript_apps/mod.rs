//! Octoscript mini-app hosting with account-bound Matrix and Octos service access.
mod matrix;
pub use rinx_miniapp_core::{InstanceId, Lease, OctosProvider, ServiceEvent};
use std::sync::LazyLock;
static AUTHORITY: LazyLock<rinx_miniapp_core::SessionAuthority> = LazyLock::new(Default::default);

pub fn invalidate_sessions() {
    AUTHORITY.invalidate();
}

pub async fn matrix_request(
    lease: Lease,
    service: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = crate::sliding_sync::get_client().ok_or("Not logged in")?;
    matrix::execute(client, lease, service, args).await
}

mod octos;
mod package;
pub mod ui;
pub use octos::KernelProvider;
pub use ui::{MiniAppsAction, MiniAppsPanelWidgetRefExt};

pub fn script_mod(vm: &mut makepad_widgets::ScriptVm) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        makepad_widgets::widget_async::register_splash_isolate_mod(|vm| {
            octoscript_widgets::design::script_mod(vm);
        });
        makepad_widgets::widget_async::register_splash_isolate_mod(|vm| {
            octoscript_widgets::kit::script_mod(vm);
        });
        makepad_widgets::widget_async::register_splash_isolate_mod(|vm| {
            octoscript_widgets::tap::script_mod(vm);
        });
        makepad_widgets::widget_async::register_splash_isolate_mod(
            makepad_widgets::splash::register_agent_module,
        );
    });
    ui::script_mod(vm);
}
