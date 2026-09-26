#![recursion_limit = "256"]

use std::{path::Path, sync::OnceLock};

use makepad_widgets::ScriptNew;
use robius_directories::ProjectDirs;

pub use makepad_widgets;

#[macro_export]
macro_rules! live {
    ($($tt:tt)*) => {
        makepad_widgets::script! { $($tt)* }
    };
}

pub type LivePtr = makepad_widgets::ScriptValue;


pub fn widget_ref_from_live_ptr(
    cx: &mut makepad_widgets::Cx,
    ptr: Option<LivePtr>,
) -> makepad_widgets::WidgetRef {
    ptr.map_or_else(makepad_widgets::WidgetRef::empty, |value| {
        cx.with_vm(|vm| makepad_widgets::WidgetRef::script_from_value(vm, value))
    })
}

pub fn view_from_live_ptr(
    cx: &mut makepad_widgets::Cx,
    ptr: Option<LivePtr>,
) -> makepad_widgets::View {
    cx.with_vm(|vm| match ptr {
        Some(value) => makepad_widgets::View::script_from_value(vm, value),
        None => makepad_widgets::View::script_new(vm),
    })
}

/// The top-level main application module.
pub mod app;
/// Rinx as an OctoSense app module.
#[cfg(feature = "octosense-module")]
pub mod module;
/// Function for loading and saving persistent application/session state.
pub mod persistence;
/// The settings screen and settings-related content/widgets.
pub mod settings;

/// Login screen
pub mod login;
/// Logout confirmation and state management
pub mod logout;
/// Core UI content: the main home screen (rooms list), room screen.
pub mod home;
/// User profile info and a user profile sliding pane.
pub mod profile;
/// A modal/dialog popup for interactive verification of users/devices.
mod verification_modal;
/// A modal/dialog popup for joining/leaving rooms, including confirming invite accept/reject.
mod join_leave_room_modal;
/// A modal/dialog popup for confirming that a user should be blocked or unblocked.
pub mod block_user_modal;
/// Shared UI components.
pub mod shared;
pub mod mini_app;
pub mod octoscript_apps;
pub mod article_app;
pub mod forwarding;
pub mod moments;
pub mod i18n;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple_fonts;
/// Generating text previews of timeline events/messages.
mod event_preview;
pub mod room;


/// All content related to TSP (Trust Spanning Protocol) wallets/identities.
#[cfg(feature = "tsp")]
pub mod tsp;
/// Dummy TSP module with placeholder widgets, for builds without TSP.
#[cfg(not(feature = "tsp"))]
pub mod tsp_dummy;

/// Support for the agent-chat / hagency coding-agent control plane.
#[cfg(feature = "agent_chat")]
pub mod agent_chat;
/// Dummy agent-chat module with placeholder widgets, for builds without agent-chat.
#[cfg(not(feature = "agent_chat"))]
pub mod agent_chat_dummy;


// Matrix stuff
pub mod sliding_sync;
pub mod space_service_sync;
pub mod avatar_cache;
pub mod room_preview_cache;
pub mod media_cache;
pub mod verification;

pub mod utils;
pub mod temp_storage;
pub mod location;
pub mod image_utils;

pub const APP_QUALIFIER: &str = "org";
pub const APP_ORGANIZATION: &str = "octosense";
pub const APP_NAME: &str = "rinx";

pub fn project_dir() -> &'static ProjectDirs {
    static RINX_PROJECT_DIRS: OnceLock<ProjectDirs> = OnceLock::new();

    RINX_PROJECT_DIRS.get_or_init(|| {
        ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_NAME)
            .expect("Failed to obtain Rinx project directory")
    })
}

pub fn app_data_dir() -> &'static Path {
    static DATA_DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
    DATA_DIR.get_or_init(|| {
        match std::env::var_os("RINX_DATA_DIR").or_else(|| std::env::var_os("ROBRIX_DATA_DIR")) {
            Some(value) => {
                let path = std::path::PathBuf::from(value);
                assert!(path.is_absolute(), "RINX_DATA_DIR (or legacy ROBRIX_DATA_DIR) must be an absolute path");
                path
            }
            None => project_dir().data_dir().to_owned(),
        }
    })
}

pub fn cache_dir() -> &'static Path {
    static CACHE_DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
    CACHE_DIR.get_or_init(|| {
        if std::env::var_os("RINX_DATA_DIR").is_some() || std::env::var_os("ROBRIX_DATA_DIR").is_some() {
            app_data_dir().join("cache")
        } else {
            project_dir().cache_dir().to_owned()
        }
    })
}
