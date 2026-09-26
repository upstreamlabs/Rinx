//! The NavigationTabBar shows a bar of icon buttons that allow the user to
//! navigate or switch between various top-level views in Rinx.
//!
//! The bar is positioned either within the left side bar (in the wide "Desktop" view mode)
//! or along the bottom of the app window (in the narrow "Mobile" view mode).
//!
//! All the buttons in this bar — including the `ProfileIcon` and the entries
//! in the embedded `SpacesBar` — are instances of the unified
//! [`NavigationBarButton`](crate::shared::navigation_bar_button::NavigationBarButton)
//! base widget, which provides hover and "selected" background animations.
//!
//! Mobile uses four persistent roots: Chats, Contacts, Discover, and Me.
//! Room navigation pushes above these roots and preserves the selected tab.
//!
//! The order in Desktop view (vertically from top to bottom) is:
//! 1. Profile/Settings
//! 2. Home
//! 3. Add/Join
//! 4. ----- separator -----
//!      SpacesBar content
//!

use makepad_widgets::*;
use serde::{Deserialize, Serialize};
use crate::{
    home::account_menu::{is_desktop_layout, AccountMenuAction},
    avatar_cache::{self, AvatarCacheEntry},
    login::login_screen::LoginAction,
    logout::logout_confirm_modal::LogoutAction,
    profile::{
        user_profile::UserProfile,
        user_profile_cache::{self, UserProfileUpdate},
    },
    home::home_screen::effective_is_desktop,
    settings::app_preferences::{AppPreferencesAction, ViewModeOverride},
    shared::{
        avatar::{AvatarState, AvatarWidgetExt},
        navigation_bar_button::{NavigationBarButton, NavigationBarButtonWidgetExt},
        styles::*,
        verification_badge::VerificationBadgeWidgetExt
    },
    sliding_sync::{current_user_id, AccountDataAction},
    utils::{self, RoomNameId},
};

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*


    // The base style definition for icon buttons in the NavigationTabBar.
    //
    // Dark navy nav-rail item (robrix2 visual spec §5.6 / RBX_NAV_* tokens):
    // transparent when idle so the navy rail shows through, a navy "pill" on
    // hover/active, plus a teal accent bar on the left edge of the *active* item
    // to echo the app-wide teal selection language. The icon itself is recolored
    // white when selected via the animator below (DrawSvg has no per-state color).
    mod.widgets.NavigationTabButton = mod.widgets.NavigationBarButton {
        width: Fill,
        height: (NAVIGATION_TAB_BAR_SIZE - 12),
        padding: (SPACE_XS),
        margin: Inset{top: 2, bottom: 2, left: (SPACE_XS), right: (SPACE_XS)},
        align: Align{x: 0.5, y: 0.5}
        flow: Down,

        draw_bg +: {
            color_hover: (RBX_NAV_ITEM_HOVER_BG)
            color_active: (RBX_NAV_ITEM_ACTIVE_BG)
            accent_color: instance((RBX_ACCENT))
            border_radius: (RBX_RADIUS_SM)

            pixel: fn() {
                let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                sdf.box(
                    self.border_inset.x + self.border_size,
                    self.border_inset.y + self.border_size,
                    self.rect_size.x - (self.border_inset.x + self.border_inset.z + self.border_size * 2.0),
                    self.rect_size.y - (self.border_inset.y + self.border_inset.w + self.border_size * 2.0),
                    max(1.0, self.border_radius)
                )
                // `fill_keep` leaves the pill in the sdf shape, and `box` unions with it,
                // so the accent bar below would flood the whole pill. `fill`/`stroke` reset it.
                if self.border_size > 0.0 {
                    sdf.fill_keep(self.get_color())
                    sdf.stroke(self.border_color, self.border_size)
                } else {
                    sdf.fill(self.get_color())
                }
                // Teal selection bar on the left edge, shown only when active.
                let bar_inset = 12.0
                sdf.box(
                    0.0,
                    bar_inset,
                    3.0,
                    self.rect_size.y - bar_inset * 2.0,
                    1.5
                )
                sdf.fill(mix(vec4(0.0, 0.0, 0.0, 0.0), self.accent_color, self.active))
                return sdf.result;
            }
        }

        icon := Icon {
            margin: 0,
            icon_walk: Walk {
                margin: 0,
                width: (RBX_ICON_LG),
                height: (RBX_ICON_LG)
            }
            draw_icon +: {
                color: (RBX_NAV_FG)
            }
        }

        // Same hover/active tracks as the base NavigationBarButton, plus the
        // icon recolor: white (RBX_NAV_FG_ACTIVE) when selected, muted
        // RBX_NAV_FG otherwise.
        animator: Animator {
            hover: {
                default: @off
                off: AnimatorState{
                    from: {all: Forward {duration: 0.15}}
                    apply: { draw_bg: {hover: 0.0} }
                }
                on: AnimatorState{
                    from: {all: Snap}
                    apply: { draw_bg: {hover: 1.0} }
                }
                down: AnimatorState{
                    from: {all: Snap}
                    apply: { draw_bg: {hover: 1.0} }
                }
            }
            active: {
                default: @off
                off: AnimatorState{
                    from: {all: Snap}
                    apply: { draw_bg: {active: 0.0} icon: { draw_icon: { color: (RBX_NAV_FG) } } }
                }
                on: AnimatorState{
                    from: {all: Snap}
                    apply: { draw_bg: {active: 1.0} icon: { draw_icon: { color: (RBX_NAV_FG_ACTIVE) } } }
                }
            }
        }
    }

    mod.widgets.ProfileIcon = #(ProfileIcon::register_widget(vm)) {
        ..mod.widgets.NavigationBarButton

        // ProfileIcon emits its own dynamic tooltip (with verification badge info)
        // from Rust, so leave the built-in tooltip text empty.
        tooltip_text: ""

        // Use the same size/shape bounds as other buttons in the NavigationTabBar
        width: Fill,
        height: (NAVIGATION_TAB_BAR_SIZE - 8)
        padding: 0,
        margin: Inset{top: 2, bottom: 2, left: (SPACE_XS), right: (SPACE_XS)},
        align: Align{ x: 0.5, y: 0.5 }

        draw_bg +: {
            color_hover: (RBX_NAV_ITEM_HOVER_BG)
            color_active: (RBX_NAV_ITEM_ACTIVE_BG)
            border_radius: (RBX_RADIUS_SM)
        }

        avatar_with_badge := View {
            width: (NAVIGATION_TAB_BAR_SIZE - 12)
            height: (NAVIGATION_TAB_BAR_SIZE - 12)
            flow: Overlay
            align: Align { x: 0.5, y: 0.5 }

            our_own_avatar := Avatar {
                width: (mod.widgets.NAVIGATION_TAB_BAR_AVATAR_SIZE)
                height: (mod.widgets.NAVIGATION_TAB_BAR_AVATAR_SIZE)
                // If no avatar picture, use white text on a dark background.
                text_view +: {
                    draw_bg.color: (COLOR_FG_DISABLED),
                    text +: {
                        draw_text +: {
                            text_style: theme.font_regular { font_size: mod.widgets.NAVIGATION_TAB_BAR_AVATAR_FONT_SIZE },
                            color: (COLOR_PRIMARY),
                        }
                    }
                }
            }

            // A Fill-sized View that aligns the badge (which is Fit-sized)
            // to the top-right corner of the wrapper. Since the wrapper is
            // larger than the avatar, the badge ends up sitting near the
            // avatar's outer top-right corner, half-overlapping the avatar.
            View {
                width: Fill,
                height: Fill,
                align: Align { x: 1.0, y: 0.0 }
                margin: Inset { left: 0, bottom: 0, top: 2, right: 2 }
                verification_badge := VerificationBadge {}
            }
        }
    }

    mod.widgets.HomeButton = mod.widgets.NavigationTabButton {
        tooltip_text: "All Rooms"
        icon +: {
            draw_icon +: { svg: (ICON_HOME) }
        }
    }

    mod.widgets.AddRoomButton = mod.widgets.NavigationTabButton {
        tooltip_text: "Add/Join Room"
        icon +: {
            icon_walk: Walk{width: 20 height: 20}
            draw_icon +: { svg: (ICON_ADD) }
        }
    }

    // The bottom rail item that opens the account menu (ported from robrix2). A plain
    // icon button, NOT a second ProfileIcon: the menu is an action, not the user's
    // avatar, and this keeps the rail's icon language consistent with Home / "+".
    // The top avatar keeps opening Settings directly.
    mod.widgets.AccountSwitcherButton = mod.widgets.NavigationTabButton {
        tooltip_text: "Account"
        icon +: {
            draw_icon +: { svg: (mod.widgets.ICON_PEOPLE) }
        }
    }

    // Built on `NavigationTabButton` so it shares the size/padding and
    // hover animation. Its toggling is independent of navigation selection,
    // so the parent never calls `set_selected` on it.
    mod.widgets.ToggleSpacesBarButton = mod.widgets.NavigationTabButton {
        tooltip_text: "Toggle Spaces"
        icon +: {
            draw_icon +: { svg: (ICON_SQUARES) }
        }
    }

    mod.widgets.Separator = LineH {
        margin: Inset{top: (SPACE_SM), bottom: (SPACE_SM), left: (SPACE_MD), right: (SPACE_MD)}
        draw_bg.color: (RBX_NAV_DIVIDER)
    }

    mod.widgets.MobileTabButton = NavigationBarButton {
        width: Fill height: 56 flow: Down spacing: 4
        padding: Inset{top: 7 bottom: 4} align: Align{x: 0.5 y: 0.5}
        draw_bg +: {color_hover: #x00000000 color_active: #x00000000 border_radius: 0}
        icon := Icon {
            icon_walk: Walk{width: 24 height: 24}
            draw_icon.color: #x191919
        }
        label := Label {
            draw_text +: {color: #x191919 text_style: theme.font_regular {font_size: 8}}
        }
    }

    mod.widgets.NavigationTabBar = #(NavigationTabBar::register_widget(vm)) {
        // Dark navy anchor rail (robrix2 visual spec §2/§5.6). SolidView fills its
        // column edge-to-edge (no rounded-SDF anti-aliased border), so the navy is
        // perfectly flush to the window's left edge AND to the rooms list.
        Desktop := SolidView {
            new_batch: true,
            flow: Down,
            align: Align{x: 0.5}
            padding: Inset{
                top: (SPACE_SM),
                bottom: (SPACE_SM + mod.widgets.SAFE_INSET_PAD_BOTTOM),
                left: (mod.widgets.SAFE_INSET_PAD_LEFT),
            }
            width: (mod.widgets.NAVIGATION_TAB_BAR_SIZE + mod.widgets.SAFE_INSET_PAD_LEFT),
            height: Fill

            show_bg: true
            draw_bg.color: (RBX_NAV_BG)

            CachedWidget {
                profile_icon := mod.widgets.ProfileIcon {}
            }
            CachedWidget {
                home_button := mod.widgets.HomeButton {}
            }
            contacts_button := mod.widgets.NavigationTabButton {
                tooltip_text: #(crate::i18n::tr("Contacts"))
                icon.draw_icon.svg: ICON_PEOPLE
            }
            moments_button := mod.widgets.NavigationTabButton {
                tooltip_text: "Moments"
                icon.draw_icon.svg: ICON_GLOBE
            }
            octoscript_apps_button := mod.widgets.NavigationTabButton {
                tooltip_text: "Mini apps"
                icon.draw_icon.svg: ICON_GLOBE
            }
            article_editor_button := mod.widgets.NavigationTabButton {
                tooltip_text: #(crate::i18n::tr("Article editor"))
                icon.draw_icon.svg: ICON_EDIT
            }
            CachedWidget {
                add_room_button := mod.widgets.AddRoomButton {}
            }

            mod.widgets.Separator {}

            CachedWidget {
                root_spaces_bar := mod.widgets.SpacesBar {}
            }

            mod.widgets.Separator {}

            // Bottom-left account menu button. The spaces bar above is height: Fill,
            // which pins this to the very bottom of the rail.
            CachedWidget {
                account_switcher_button := mod.widgets.AccountSwitcherButton {}
            }
        }

        Mobile := SolidView {
            new_batch: true flow: Right align: Align{y: 0.5}
            width: Fill height: (56 + mod.widgets.SAFE_INSET_PAD_BOTTOM)
            padding: Inset{bottom: (mod.widgets.SAFE_INSET_PAD_BOTTOM)}
            draw_bg.color: #xf7f7f7
            chats_tab := mod.widgets.MobileTabButton {
                icon.draw_icon.svg: crate_resource("self://resources/icons/double_chat.svg")
                label.text: #(crate::i18n::tr("Chats")) label.i18n_text: "Chats"
            }
            contacts_tab := mod.widgets.MobileTabButton {icon.draw_icon.svg: ICON_PEOPLE label.text: #(crate::i18n::tr("Contacts")) label.i18n_text: "Contacts"}
            discover_tab := mod.widgets.MobileTabButton {icon.draw_icon.svg: ICON_GLOBE label.text: #(crate::i18n::tr("Discover")) label.i18n_text: "Discover"}
            me_tab := mod.widgets.MobileTabButton {icon.draw_icon.svg: crate_resource("self://resources/icons/person.svg") label.text: #(crate::i18n::tr("Me")) label.i18n_text: "Me"}
        }
    }
}

/// The icon in the NavigationTabBar that shows the user's avatar.
///
/// This widget serves as both the visual user-avatar indicator AND the
/// entry point to the SettingsScreen — clicking it opens settings and
/// marks this button as the currently-selected navigation tab.
///
/// `ProfileIcon` derefs into [`NavigationBarButton`], so it inherits the
/// hover/selected background animations and emits
/// `NavigationBarButtonAction::Clicked` on tap (handled by `NavigationTabBar`
/// to navigate to the Settings screen). Its dynamic tooltip (which includes
/// verification badge state) is emitted by this widget itself rather than
/// using `NavigationBarButton`'s built-in `tooltip_text`.
#[derive(Script, Widget)]
pub struct ProfileIcon {
    #[deref] inner: NavigationBarButton,
    #[rust] own_profile: Option<UserProfile>,
}

impl ScriptHook for ProfileIcon {
    fn on_after_reload(&mut self, vm: &mut ScriptVm) {
        vm.with_cx_mut(|cx| {
            if self.own_profile.is_none() {
                self.own_profile = get_own_profile(cx);
            }
        });
    }
}

impl Widget for ProfileIcon {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        if self.own_profile.is_none() {
            self.own_profile = get_own_profile(cx);
        }

        // A UI Signal indicates that a user profile or avatar may have been updated.
        if let Event::Signal = event {
            let mut needs_redraw = false;
            // Refetch our profile if we don't have it yet.
            if self.own_profile.is_none() {
                user_profile_cache::process_user_profile_updates(cx);
                self.own_profile = get_own_profile(cx);
                needs_redraw = true;
            }
            // If we're waiting for an avatar image, process avatar updates.
            if let Some(p) = self.own_profile.as_mut() && p.avatar_state.uri().is_some() {
                avatar_cache::process_avatar_updates(cx);
                let new_data = p.avatar_state.update_from_cache(cx);
                needs_redraw |= new_data.is_some();
                if new_data.is_some() {
                    user_profile_cache::enqueue_user_profile_update(
                        UserProfileUpdate::UserProfileOnly(p.clone())
                    );
                }
            }
            if needs_redraw {
                self.inner.redraw(cx);
            }
        }

        // Handle actions related to the currently-logged-in user account,
        // such as changing their avatar, display name, etc.
        if let Event::Actions(actions) = event {
            for action in actions {
                if let Some(LoginAction::LoginSuccess) = action.downcast_ref() {
                    self.own_profile = get_own_profile(cx);
                    self.inner.redraw(cx);
                    continue;
                }

                if let Some(LogoutAction::ClearAppState { .. }) = action.downcast_ref() {
                    self.own_profile = None;
                    self.inner.redraw(cx);
                    continue;
                }

                // Handle account data changes (e.g., avatar updated/removed)
                match action.downcast_ref() {
                    Some(AccountDataAction::AvatarChanged(None)) => {
                        // Update both this widget's local profile info and the user profile cache.
                        if let Some(p) = self.own_profile.as_mut() {
                            p.avatar_state = AvatarState::Known(None);
                            user_profile_cache::enqueue_user_profile_update(
                                UserProfileUpdate::UserProfileOnly(p.clone())
                            );
                            self.inner.redraw(cx);
                        }
                        continue;
                    }
                    Some(AccountDataAction::AvatarChanged(Some(new_uri))) => {
                        if let Some(p) = self.own_profile.as_mut() {
                            p.avatar_state = AvatarState::Known(Some(new_uri.clone()));
                            p.avatar_state.update_from_cache(cx);
                            user_profile_cache::enqueue_user_profile_update(
                                UserProfileUpdate::UserProfileOnly(p.clone())
                            );
                            self.inner.redraw(cx);
                        }
                        continue;
                    }
                    Some(AccountDataAction::AvatarChangeFailed(_)) => {
                        // this is only handled in the account settings screen
                        continue;
                    }
                    Some(AccountDataAction::DisplayNameChanged(new_display_name)) => {
                        if let Some(p) = self.own_profile.as_mut() {
                            p.username = new_display_name.clone();
                            user_profile_cache::enqueue_user_profile_update(
                                UserProfileUpdate::UserProfileOnly(p.clone())
                            );
                            self.inner.redraw(cx);
                        }
                        continue;
                    }
                    Some(AccountDataAction::DisplayNameChangeFailed(_)) => {
                        // this is only handled in the account settings screen
                        continue;
                    }
                    _ => {}
                }
            }
        }

        // Forward to the inner NavigationBarButton, which handles hover/selected
        // animations and emits `NavigationBarButtonAction::Clicked` on tap.
        self.inner.handle_event(cx, event, scope);

        // Emit ProfileIcon's own dynamic tooltip (which includes verification
        // badge state). This is in addition to (not instead of) the inner
        // button's hit handling: calling `event.hits()` twice on the same area
        // is safe in Makepad — both calls return the same hit.
        let area = self.inner.view.area();
        match event.hits(cx, area) {
            Hit::FingerLongPress(_) | Hit::FingerHoverIn(_) => {
                let (verification_str, bg_color) = self.inner.view
                    .verification_badge(cx, ids!(verification_badge))
                    .tooltip_content();
                let text = self.own_profile.as_ref().map_or_else(
                    || String::from("Not logged in (or disconnected).\n\nClick/tap to access all settings."),
                    |p| crate::i18n::format("Logged in {verification_str}as \"{0}\".\n\nClick/tap to access all settings.", &[("verification_str", (verification_str).to_string()), ("0", (p.displayable_name()).to_string())]),
                );
                let mut options = CalloutTooltipOptions {
                    position: if effective_is_desktop(cx) { TooltipPosition::Right } else { TooltipPosition::Top },
                    ..Default::default()
                };
                if let Some(c) = bg_color {
                    options.bg_color = c;
                }
                cx.widget_action(
                    self.widget_uid(),
                    TooltipAction::HoverIn {
                        text,
                        widget_rect: area.rect(cx),
                        options,
                    },
                );
            }
            Hit::FingerHoverOut(_) => {
                cx.widget_action(self.widget_uid(),  TooltipAction::HoverOut);
            }
            _ => { }
        };
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let our_own_avatar = self.inner.view.avatar(cx, ids!(our_own_avatar));
        let Some(own_profile) = self.own_profile.as_ref() else {
            // If we don't have a profile, default to an unknown avatar.
            our_own_avatar.show_text(
                cx,
                Some(COLOR_FG_DISABLED),
                None, // don't make this avatar clickable; we handle clicks on this ProfileIcon widget directly.
                "",
            );
            return self.inner.draw_walk(cx, scope, walk);
        };

        let mut drew_avatar = false;
        if let Some(avatar_image) = own_profile.avatar_state.image() {
            drew_avatar = our_own_avatar.show_image(
                cx,
                None, // don't make this avatar clickable; we handle clicks on this ProfileIcon widget directly.
                |cx, img| utils::load_avatar_image(&img, cx, avatar_image),
            ).is_ok();
        }
        if !drew_avatar {
            our_own_avatar.show_text(
                cx,
                Some(crate::shared::design_tokens::RBX_IDENTITY_TEAL),
                None, // don't make this avatar clickable; we handle clicks on this ProfileIcon widget directly.
                own_profile.displayable_name(),
            );
        }

        self.inner.draw_walk(cx, scope, walk)
    }
}

impl ProfileIconRef {
    /// Visually marks this `ProfileIcon` as selected (or not).
    /// Forwards to [`NavigationBarButton::set_selected`].
    pub fn set_selected(&self, cx: &mut Cx, is_selected: bool) {
        let Some(mut inner) = self.borrow_mut() else { return };
        inner.inner.set_selected(cx, is_selected);
    }
}


/// The tab bar with buttons that navigate through top-level app pages.
///
/// * In the "desktop" (wide) layout, this is a vertical bar on the left.
/// * In the "mobile" (narrow) layout, this is a horizontal bar on the bottom.
#[derive(Script, Widget)]
pub struct NavigationTabBar {
    #[deref] view: AdaptiveView,

    #[rust] is_spaces_bar_shown: bool,

    /// The most recently applied view-mode override,
    #[rust] applied_view_mode: ViewModeOverride,

    /// The tab currently visually marked as selected.
    #[rust] selected_tab: SelectedTab,
    #[rust] mobile_palette: Option<(WidgetUid, SelectedTab)>,
}

impl ScriptHook for NavigationTabBar {
    fn on_after_new(&mut self, vm: &mut ScriptVm) {
        vm.with_cx_mut(|cx| {
            self.apply_selected_tab(cx, None);
        });
    }

    fn on_after_reload(&mut self, vm: &mut ScriptVm) {
        vm.with_cx_mut(|cx| {
            self.mobile_palette = None;
            self.apply_selected_tab(cx, None);
        });
    }
}

impl NavigationTabBar {
    /// Installs a variant selector on our root `AdaptiveView` that honors the
    /// given [`ViewModeOverride`] preference. `Automatic` falls back to the
    /// default width-based selector.
    fn apply_view_mode(&mut self, mode: ViewModeOverride) {
        self.view.set_variant_selector(mode.variant_selector());
        self.applied_view_mode = mode;
    }

    /// Updates which navigation tab button is visually marked as selected,
    /// enforcing mutual exclusion across all buttons (like a radio button group).
    ///
    /// If `tab` is `None`, the existing selection is re-applied without changing it.
    fn apply_selected_tab(&mut self, cx: &mut Cx, tab: Option<SelectedTab>) {
        if let Some(t) = tab {
            self.selected_tab = t;
        }
        let home    = self.view.navigation_bar_button(cx, ids!(home_button));
        let contacts = self.view.navigation_bar_button(cx, ids!(contacts_button));
        let add     = self.view.navigation_bar_button(cx, ids!(add_room_button));
        let profile = self.view.profile_icon(cx, ids!(profile_icon));
        home.set_selected(cx, self.selected_tab == SelectedTab::Home);
        contacts.set_selected(cx, self.selected_tab == SelectedTab::Contacts);
        add.set_selected(cx, self.selected_tab == SelectedTab::AddRoom);
        profile.set_selected(cx, matches!(self.selected_tab, SelectedTab::Settings | SelectedTab::Me));
        let chats_uid = self.view.navigation_bar_button(cx, ids!(chats_tab)).widget_uid();
        let palette = (chats_uid, self.selected_tab.clone());
        let update_palette = self.mobile_palette.as_ref() != Some(&palette);
        for (id, selected) in [
            (ids!(chats_tab), matches!(self.selected_tab, SelectedTab::Home | SelectedTab::Space {..})),
            (ids!(contacts_tab), self.selected_tab == SelectedTab::Contacts),
            (ids!(discover_tab), matches!(self.selected_tab, SelectedTab::Discover | SelectedTab::AddRoom)),
            (ids!(me_tab), matches!(self.selected_tab, SelectedTab::Me | SelectedTab::Settings)),
        ] {
            let button = self.view.navigation_bar_button(cx, id);
            button.set_selected(cx, selected);
            if update_palette && !button.is_empty() {
                let color = if selected { vec4(0.027, 0.757, 0.376, 1.0) } else { vec4(0.098, 0.098, 0.098, 1.0) };
                let mut icon = self.view.icon(cx, &[id[0], id!(icon)]);
                script_apply_eval!(cx, icon, {draw_icon +: {color: #(color)}});
                self.view.label(cx, &[id[0], id!(label)]).set_text_color(cx, color);
            }
        }
        self.mobile_palette = Some(palette);
    }
}

impl Widget for NavigationTabBar {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);

        if let Event::Actions(actions) = event {
            for (id, tab) in [
                (ids!(chats_tab), SelectedTab::Home),
                (ids!(contacts_tab), SelectedTab::Contacts),
                (ids!(contacts_button), SelectedTab::Contacts),
                (ids!(discover_tab), SelectedTab::Discover),
                (ids!(me_tab), SelectedTab::Me),
            ] {
                if self.view.navigation_bar_button(cx, id).clicked(actions) {
                    cx.action(NavigationBarAction::GoToTab(tab));
                }
            }
            // Handle clicks on each of the navigation tab buttons.
            // Each click both updates the visual selection and emits the
            // corresponding `NavigationBarAction` for downstream handling.
            if self.view.navigation_bar_button(cx, ids!(home_button)).clicked(actions) {
                self.apply_selected_tab(cx, Some(SelectedTab::Home));
                cx.action(NavigationBarAction::GoToHome);
            }
            else if self.view.navigation_bar_button(cx, ids!(add_room_button)).clicked(actions) {
                self.apply_selected_tab(cx, Some(SelectedTab::AddRoom));
                cx.action(NavigationBarAction::GoToAddRoom);
            }
            else {
                // ProfileIcon's inner NavigationBarButton emits the click action,
                // and ProfileIcon derefs into it, so the same `clicked()` check works.
                let profile_icon_ref = self.view.profile_icon(cx, ids!(profile_icon));
                let profile_clicked = profile_icon_ref
                    .borrow()
                    .is_some_and(|p| p.inner.clicked(actions));
                if profile_clicked {
                    self.apply_selected_tab(cx, Some(SelectedTab::Settings));
                    cx.action(NavigationBarAction::OpenSettings);
                }
            }

            // The bottom account button opens the AccountMenu, anchored so the card's
            // BOTTOM-left sits at the button's bottom-right (the App grows it upward).
            if is_desktop_layout(cx)
                && self.view.navigation_bar_button(cx, ids!(account_switcher_button)).clicked(actions)
            {
                let rect = self.view.widget(cx, ids!(account_switcher_button)).area().rect(cx);
                cx.action(AccountMenuAction::Open {
                    pos: dvec2(rect.pos.x + rect.size.x + 4.0, rect.pos.y + rect.size.y),
                });
            }

            if self.view.navigation_bar_button(cx, ids!(moments_button)).clicked(actions) {
                cx.action(crate::moments::ui::MomentsAction::Open {author: None});
            }
            if self.view.navigation_bar_button(cx,ids!(octoscript_apps_button)).clicked(actions) {cx.action(crate::octoscript_apps::MiniAppsAction::Open);}
            if self.view.navigation_bar_button(cx, ids!(article_editor_button)).clicked(actions) {
                cx.action(crate::article_app::ArticleAction::Open);
            }
            if self.view.navigation_bar_button(cx, ids!(toggle_spaces_bar_button)).clicked(actions) {
                self.is_spaces_bar_shown = !self.is_spaces_bar_shown;
                cx.action(NavigationBarAction::ToggleSpacesBar);
            }

            for action in actions {
                // If another widget programmatically selected a new tab,
                // update our buttons' visual selection state accordingly.
                if let Some(NavigationBarAction::TabSelected(tab)) = action.downcast_ref() {
                    self.apply_selected_tab(cx, Some(tab.clone()));
                    continue;
                }

                // Upon login (mostly re-login), go back to the home tab
                // because the profile/settings tab will have been selected upon logout.
                if let Some(LoginAction::LoginSuccess) = action.downcast_ref() {
                    self.apply_selected_tab(cx, Some(SelectedTab::Home));
                    cx.action(NavigationBarAction::GoToHome);
                    continue;
                }

                if let Some(AppPreferencesAction::ViewModeChanged(new_mode)) = action.downcast_ref() {
                    if *new_mode != self.applied_view_mode {
                        self.apply_view_mode(*new_mode);
                        self.view.redraw(cx);
                    }
                }
            }
        }
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        if let Some(state) = scope.data.get::<crate::app::AppState>() {
            self.selected_tab = state.selected_tab.clone();
        }
        self.apply_selected_tab(cx, None);
        let step = self.view.draw_walk(cx, scope, walk);
        // AdaptiveView creates its variant during draw. Apply again after that
        // first draw so a new mobile bar doesn't remain visually unselected
        // until a tap or an unrelated sync signal arrives.
        self.apply_selected_tab(cx, None);
        for (id, tooltip) in [
            (ids!(contacts_button), crate::i18n::tr("Contacts")),
            (ids!(article_editor_button), crate::i18n::tr("Article editor")),
        ] {
            if let Some(mut button) = self.view.navigation_bar_button(cx, id).borrow_mut() {
                button.set_tooltip_text(tooltip);
            }
        }
        step
    }
}


/// Which top-level view is currently shown, and which navigation tab is selected.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectedTab {
    #[default]
    Home,
    Contacts,
    Discover,
    Me,
    AddRoom,
    Settings,
    // AlertsInbox,
    Space { space_name_id: RoomNameId },
}
impl SelectedTab {
    /// Updates this tab's space name if it refers to the same space.
    ///
    /// Returns `true` if the name was changed.
    pub fn update_space_name(&mut self, new_space_name: &RoomNameId) -> bool {
        let SelectedTab::Space { space_name_id } = self else { return false };
        if space_name_id.room_id() != new_space_name.room_id()
            || space_name_id.display_name() == new_space_name.display_name()
        {
            return false;
        }
        *space_name_id = new_space_name.clone();
        true
    }
}


/// Actions for navigating through the top-level views of the app,
/// e.g., when the user clicks/taps on a button in the NavigationTabBar.
///
/// ## Tip: you only want to handle `TabSelected`
/// The most important variant is `TabSelected`, which is most likely the action
/// that you want to handle in other widgets, if you care about which
/// top-level navigation tab is currently selected.
/// This is because the `TabSelected` variant will always occur even if the
/// other actions do not occur --- for example, if the user chooses to jump
/// to a different view (or back to a previous view) without explicitly clicking
/// a navigation tab button, e.g., via a keyboard shortcut, or programmatically.
///
/// Only one widget, the `HomeScreen`, should emit the `TabSelected` action.
/// All other widgets should handle only that action in order to ensure
/// consistent behavior.
///
/// ## More details
/// There are 3 kinds of actions within this one enum:
/// 1. "Leading-edge" ("request") actions emitted by the NavigationTabBar
///    when the user selects a particular button/space.
///    * Includes `GoToHome`, `GoToAddRoom`, `GoToSpace`, `OpenSettings`, `CloseSettings`.
/// 2. "Trailing-edge" ("response") actions that are emitted by the `HomeScreen` widget
///    in response to a leading-edge action.
///    * This includes only the `TabSelected` variant.
///    * This is what all other widgets should handle if they want/need to respond
///      to changes in the top-level app-wide navigation selection.
/// 3. Other actions that aren't requests/responses to navigate to a different view.
///    * This only includes the `ToggleSpacesBar` variant.
#[derive(Debug, PartialEq, Eq)]
pub enum NavigationBarAction {
    /// Select a top-level root while keeping its page state.
    GoToTab(SelectedTab),
    /// Go to the main rooms content view.
    GoToHome,
    /// Go the add/join/explore room view.
    GoToAddRoom,
    /// Leave Explore Rooms and restore the tab that opened it.
    CloseAddRoom,
    /// Go to the Settings view (open the `SettingsScreen`).
    OpenSettings,
    /// Open the current user's mobile Personal Information page.
    OpenOwnProfile,
    /// Close the Settings view (`SettingsScreen`), returning to the previous view.
    CloseSettings,
    /// Go the space screen for the given space.
    GoToSpace { space_name_id: RoomNameId },

    // TODO: add GoToAlertsInbox, once we add that button/screen

    /// The given tab was selected as the active top-level view.
    /// This is needed to ensure that the proper tab is marked as selected. 
    TabSelected(SelectedTab),
    /// Toggle whether the SpacesBar is shown, i.e., show/hide it.
    /// This is only applicable in the Mobile view mode, because the SpacesBar
    /// is always shown in Desktop view mode.
    ToggleSpacesBar,
}


/// Returns the current user's profile and avatar, if available.
pub fn get_own_profile(cx: &mut Cx) -> Option<UserProfile> {
    let mut own_profile = None;
    if let Some(own_user_id) = current_user_id() {
        let avatar_uri_to_fetch = user_profile_cache::with_user_profile(
            cx,
            own_user_id,
            None,
            true,
            |new_profile, _rooms| {
                let avatar_uri_to_fetch = new_profile.avatar_state.uri().cloned();
                own_profile = Some(new_profile.clone());
                avatar_uri_to_fetch
            },
        );
        // If we have an avatar URI to fetch, try to fetch it.
        if let Some(Some(avatar_uri)) = avatar_uri_to_fetch {
            if let AvatarCacheEntry::Loaded(data) = avatar_cache::get_or_fetch_avatar(cx, &avatar_uri) {
                if let Some(p) = own_profile.as_mut() {
                    p.avatar_state = AvatarState::Loaded((avatar_uri.clone(), data).into());
                    // Update the user profile cache with the new avatar data.
                    user_profile_cache::enqueue_user_profile_update(
                        UserProfileUpdate::UserProfileOnly(p.clone())
                    );
                }
            }
        }
    }

    own_profile
}
