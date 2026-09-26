use makepad_widgets::*;
use super::back_swipe::BackSwipe;

use crate::{
    app::{AppState, AppStateAction, SelectedRoom},
    home::{
        mobile_chat_info::MobileChatInfoWidgetRefExt,
        rooms_list_header::RoomsListHeaderAction,
        invite_screen::InviteScreenWidgetRefExt,
        navigation_tab_bar::{NavigationBarAction, SelectedTab},
        room_screen::RoomScreenWidgetRefExt,
        rooms_list::{AcceptedInviteKind, RoomsListAction},
        space_lobby::SpaceLobbyScreenWidgetRefExt,
        spaces_bar::SpacesBarAction,
    },
    settings::{
        app_preferences::{AppPreferencesGlobal, AppPreferencesAction, ViewModeOverride},
        settings_screen::SettingsScreenWidgetRefExt,
    },
    profile::user_profile::UserProfileSlidingPaneWidgetRefExt,
    shared::room_filter_input_bar::{MainFilterAction, RoomFilterInputBarWidgetExt},
    shared::mention_popup::MentionablePopupRef,
    utils::RoomNameId,
};

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*


    // Defines the total height of the StackNavigationView's header.
    // This has to be set in multiple places because of how StackNavigation
    // uses an Overlay view internally.
    mod.widgets.STACK_VIEW_HEADER_HEIGHT = 45

    // A reusable base for StackNavigationView children in the mobile layout.
    // Each specific screen view (room, invite, space lobby) extends this
    // and places its own screen widget inside the body.
    mod.widgets.RobrixStackNavigationView = StackNavigationView {
        width: Fill, height: Fill
        draw_bg.color: (COLOR_PRIMARY)
        header +: {
            height: (mod.widgets.STACK_VIEW_HEADER_HEIGHT),
            padding: 0
            align: Align{y: 0.5}

            show_bg: true
            draw_bg +: {
                color: #xededed
                pixel: fn() {return self.color}
            }

            content +: {
                height: (mod.widgets.STACK_VIEW_HEADER_HEIGHT)
                align: Align{y: 0.5}
                padding: Inset{
                    left: (mod.widgets.SAFE_INSET_PAD_LEFT),
                    right: (mod.widgets.SAFE_INSET_PAD_RIGHT),
                }
                button_container +: {
                    padding: 0,
                    margin: 0
                    left_button +: {
                        width: 48 height: (mod.widgets.STACK_VIEW_HEADER_HEIGHT)
                        padding: 0 margin: 0
                        align: Align{x: 0.5 y: 0.5}
                        draw_icon +: {svg: ICON_CHEVRON_LEFT color: (ROOM_NAME_TEXT_COLOR)}
                        icon_walk: Walk{width: 8 height: 14}
                        spacing: 0
                        text: ""
                    }
                }
                title_container +: {
                    // padding: Inset{top: 8}
                    title +: {
                        draw_text +: {
                            color: #x191919
                            text_style: theme.font_bold {font_size: 12.5}
                        }
                    }
                }
            }
        }
        body +: {
            // The top margin leaves room for the stack nav header.
            // The other padding is for safe inset areas.
            margin: Inset{top: (mod.widgets.STACK_VIEW_HEADER_HEIGHT)}
            padding: Inset{
                left: (mod.widgets.SAFE_INSET_PAD_LEFT),
                right: (mod.widgets.SAFE_INSET_PAD_RIGHT),
                bottom: (mod.widgets.SAFE_INSET_PAD_BOTTOM),
            }
        }
    }

    // A wrapper view around the SpacesBar that lets us show/hide it via animation.
    mod.widgets.SpacesBarWrapper = set_type_default() do #(SpacesBarWrapper::register_widget(vm)) {
        ..mod.widgets.RoundedShadowView

        width: Fill,
        height: (NAVIGATION_TAB_BAR_SIZE)
        margin: Inset{
            left: (4.0 + mod.widgets.SAFE_INSET_PAD_LEFT),
            right: (4.0 + mod.widgets.SAFE_INSET_PAD_RIGHT),
        }
        show_bg: true
        draw_bg +: {
            color: (COLOR_PRIMARY_DARKER)
            border_radius: 4.0
            border_size: 0.0
            shadow_color: #0005
            shadow_radius: 15.0
            shadow_offset: vec2(1.0, 0.0)
        }

        CachedWidget {
            root_spaces_bar := mod.widgets.SpacesBar {}
        }

        animator: Animator{
            spaces_bar_animator: {
                default: @hide
                show: AnimatorState{
                    redraw: true
                    from: { all: Forward { duration: (mod.widgets.SPACES_BAR_ANIMATION_DURATION_SECS) } }
                    apply: { height: (NAVIGATION_TAB_BAR_SIZE),  draw_bg: { shadow_color: #x00000055 } }
                }
                hide: AnimatorState{
                    redraw: true
                    from: { all: Forward { duration: (mod.widgets.SPACES_BAR_ANIMATION_DURATION_SECS) } }
                    apply: { height: 0,  draw_bg: { shadow_color: (COLOR_TRANSPARENT) } }
                }
            }
        }
    }

    // The home screen widget contains the main content:
    // rooms list, room screens, and the settings screen as an overlay.
    // It adapts to both desktop and mobile layouts.
    mod.widgets.HomeScreen = #(HomeScreen::register_widget(vm)) {
        main_adaptive_view := AdaptiveView {
            // NOTE: within each of these sub views, we used `CachedWidget` wrappers
            //       to ensure that there is only a single global instance of each
            //       of those widgets, which means they maintain their state
            //       across transitions between the Desktop and Mobile variant.
            Desktop := SolidView {
                width: Fill, height: Fill
                flow: Right
                align: Align{x: 0.0, y: 0.0}
                padding: 0,
                margin: 0,

                show_bg: true
                draw_bg +: {
                    color: (COLOR_SECONDARY)
                }

                // On the left, show the navigation tab bar vertically.
                CachedWidget {
                    navigation_tab_bar := mod.widgets.NavigationTabBar {}
                }

                // To the right of that, we use the PageFlip widget to show either
                // the main desktop UI or the settings screen.
                home_screen_page_flip := PageFlip {
                    width: Fill, height: Fill
                    // We only need bottom and right-side padding,
                    // as the others are handled by the parent widget
                    // or by the navigation bar.
                    padding: Inset{
                        bottom: (mod.widgets.SAFE_INSET_PAD_BOTTOM),
                        right: (mod.widgets.SAFE_INSET_PAD_RIGHT),
                    }

                    lazy_init: true,
                    active_page: @home_page

                    home_page := View {
                        width: Fill, height: Fill
                        flow: Down

                        View {
                            width: Fill,
                            height: 39,
                            flow: Right
                            padding: Inset{top: 2, bottom: 2}
                            // The negative left/right margins compensate for the gray border,
                            // such that the inner white input part is aligned with other elements.
                            margin: Inset{left: -1.5, right: -1.5}
                            spacing: 2
                            align: Align{y: 0.5}

                            CachedWidget {
                                room_filter_input_bar := RoomFilterInputBar {}
                            }

                            // Hide this until it's implemented.
                            // search_messages_button := SearchMessagesButton {
                            //     // make this button match/align with the RoomFilterInputBar
                            //     height: 32.5,
                            //     margin: Inset{right: 2}
                            // }
                        }

                        mod.widgets.MainDesktopUI {}
                    }

                    contacts_page := MobileHub {kind: 0}
                    discover_page := MobileHub {kind: 1}
                    me_page := MobileHub {kind: 2}

                    settings_page := RoundedView {
                        width: Fill, height: Fill
                        // This weird margin is just to make it line up with the home_page content.
                        margin: Inset{top: 3, left: 1, right: 0, bottom: 0}
                        show_bg: true,
                        draw_bg +: {
                            color: (COLOR_PRIMARY)
                            border_radius: 4.0
                        }

                        CachedWidget {
                            settings_screen := mod.widgets.SettingsScreen {}
                        }
                    }

                    add_room_page := RoundedView {
                        width: Fill, height: Fill
                        // This weird margin is just to make it line up with the home_page content.
                        margin: Inset{top: 3, left: 1, right: 0, bottom: 0}
                        show_bg: true,
                        draw_bg +: {
                            color: (COLOR_PRIMARY)
                            border_radius: 4.0
                        }

                        CachedWidget {
                            add_room_screen := mod.widgets.AddRoomScreen {}
                        }
                    }
                }
            }

            Mobile := SolidView {
                width: Fill, height: Fill
                flow: Down

                show_bg: true
                draw_bg.color: (COLOR_PRIMARY)

                view_stack := StackNavigation {
                    root_view +: {
                        flow: Down
                        width: Fill, height: Fill

                        // At the top of the root view, we use the PageFlip widget to show either
                        // the main list of rooms or the settings screen.
                        home_screen_page_flip := PageFlip {
                            width: Fill, height: Fill
                            padding: Inset{
                                left: (mod.widgets.SAFE_INSET_PAD_LEFT),
                                right: (mod.widgets.SAFE_INSET_PAD_RIGHT),
                            }

                            lazy_init: true,
                            active_page: @home_page

                            home_page := View {
                                width: Fill, height: Fill
                                // Note: while the other page views have top padding, we do NOT add that here
                                // because it is added in the `RoomsSideBar`'s `RoundedShadowView` itself.
                                flow: Down

                                mod.widgets.RoomsSideBar {}
                            }

                            contacts_page := MobileHub {kind: 0}
                            discover_page := MobileHub {kind: 1}
                            me_page := MobileHub {kind: 2}

                            settings_page := View {
                                width: Fill, height: Fill

                                CachedWidget {
                                    settings_screen := mod.widgets.SettingsScreen {}
                                }
                            }

                            add_room_page := View {
                                width: Fill, height: Fill

                                CachedWidget {
                                    add_room_screen := mod.widgets.AddRoomScreen {}
                                }
                            }
                        }

                        // Keep the legacy rail's sync consumer cached across adaptive
                        // layouts. Mobile membership navigation now lives inside Chats.
                        mobile_spaces_navigation := View {
                            width: Fill height: Fit visible: false
                            CachedWidget {
                                spaces_bar_wrapper := mod.widgets.SpacesBarWrapper {}
                            }
                        }

                        mobile_navigation := View {
                            width: Fill height: Fit
                            CachedWidget {navigation_tab_bar := mod.widgets.NavigationTabBar {}}
                        }
                    }

                    stack_templates: {
                        RoomScreenStackNavigationView := mod.widgets.RobrixStackNavigationView {
                            header +: {content +: {
                                title_container +: {padding: Inset{left: 62 right: 54}}
                                info_controls := View {
                                    width: Fill height: 45 align: Align{x: 1 y: 0.5}
                                    chat_info_button := RobrixNeutralIconButton {
                                        width: 48 height: 44 padding: 12
                                        text: "···"
                                        draw_text +: {color: #x191919 text_style: theme.font_bold {font_size: 18}}
                                        draw_bg +: {color: #x00000000 color_hover: #x00000000 border_size: 0}
                                        icon_walk: Walk{width: 0 height: 0}
                                    }
                                }
                            }}
                            body +: {
                                room_screen := mod.widgets.RoomScreen {
                                    room_screen_wrapper +: {
                                        draw_bg.color: #xededed
                                        timeline_and_input_bar +: {
                                            room_input_bar := mod.widgets.MobileRoomInputBar {}
                                        }
                                    }
                                }
                            }
                        }

                        ChatInfoStackNavigationView := mod.widgets.RobrixStackNavigationView {
                            body +: {chat_info := mod.widgets.MobileChatInfo {}}
                        }

                        InviteScreenStackNavigationView := mod.widgets.RobrixStackNavigationView {
                            body +: {
                                invite_screen := mod.widgets.InviteScreen {}
                            }
                        }

                        SpaceLobbyScreenStackNavigationView := mod.widgets.RobrixStackNavigationView {
                            body +: {
                                space_lobby_screen := mod.widgets.SpaceLobbyScreen {}
                            }
                        }
                    }
                }
            }
        }
    }
}


/// A simple wrapper around the SpacesBar that allows us to animate showing or hiding it.
#[derive(Script, Widget, Animator)]
pub struct SpacesBarWrapper {
    #[source] source: ScriptObjectRef,
    #[deref] view: View,
    #[apply_default] animator: Animator,
}

impl ScriptHook for SpacesBarWrapper {
    fn on_after_apply(
        &mut self,
        vm: &mut ScriptVm,
        apply: &Apply,
        scope: &mut Scope,
        _value: ScriptValue,
    ) {
        // When the widget tree is re-applied (e.g. after a preference change),
        // the deref `view` resets its height to the DSL default,
        // which clashes with whatever animator state we were in (shown, hidden).
        // Thus, we re-apply the current animator state to prevent a hidden SpacesBar
        // from briefly becoming shown before being hidden again.
        // Note that we can't just call `animator_cut` cuz that uses the script VM
        // which is unavailable from this `on_after_apply`
        if !apply.is_script_reapply() {
            return;
        }
        if let Some(state_apply) = self
            .animator
            .current_state_apply(live_id!(spaces_bar_animator))
        {
            self.script_apply(vm, &Apply::Animate, scope, state_apply.into());
        }
    }
}

impl Widget for SpacesBarWrapper {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        if self.animator_handle_event(cx, event).must_redraw() {
            self.redraw(cx);
        }
        self.view.handle_event(cx, event, scope);
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        self.view.draw_walk(cx, scope, walk)
    }
}

impl SpacesBarWrapperRef {
    /// Shows or hides the spaces bar by animating it in or out.
    fn show_or_hide(&self, cx: &mut Cx, show: bool) {
        let Some(mut inner) = self.borrow_mut() else { return };
        if show {
            inner.animator_play(cx, ids!(spaces_bar_animator.show));
        } else {
            inner.animator_play(cx, ids!(spaces_bar_animator.hide));
        }
        inner.redraw(cx);
    }
}


/// The variant that the main `AdaptiveView` last selected,
/// or `None` if it hasn't made its first selection yet.
///
/// Don't query this directly, instead call [`effective_is_desktop()`].
///
/// The inner value should only be modified by [`HomeScreen::apply_view_mode()`].
#[derive(Default)]
pub struct MainViewIsDesktop(Option<bool>);

/// An action emitted when the main `AdaptiveView` switches between Desktop and Mobile.
#[derive(Debug)]
pub struct MainViewVariantChangedAction;

/// Returns whether the UI is currently showing the wide "desktop" layout.
pub fn effective_is_desktop(cx: &mut Cx) -> bool {
    cx.global::<MainViewIsDesktop>().0
        .unwrap_or(true) // Before the first selection, default to desktop mode
}


#[derive(Script, Widget)]
pub struct HomeScreen {
    #[rust] back_swipe: BackSwipe,
    #[rust] mobile_detail_open: bool,
    /// Chat Info temporarily covers the selected room, whose state is saved normally.
    #[rust] mobile_chat_info: Option<LiveId>,
    #[deref] view: View,

    /// The previously-selected navigation tab, used to determine which tab
    /// and top-level view we return to after closing the settings screen.
    ///
    /// Note that the current selected tap is stored in `AppState` so that
    /// other widgets can easily access it.
    #[rust] previous_selection: SelectedTab,
    #[rust] explore_return_tab: SelectedTab,
    #[rust] is_spaces_bar_shown: bool,

    /// A history of previously-selected screens for mobile stack navigation.
    /// When a view is popped off the stack, the previous `selected_room` is restored.
    #[rust] mobile_screen_history: Vec<SelectedRoom>,

    /// The most recently applied view-mode override, used to short-circuit
    /// redundant `AdaptiveView` selector reinstalls when an
    /// [`AppPreferencesAction::ViewModeChanged`] action repeats the current
    /// value (e.g., the unconditional broadcast on app-state restore).
    #[rust] applied_view_mode: ViewModeOverride,

    /// The last effective AdaptiveView mode we observed. `Some(true)` means desktop mode.
    #[rust] last_effective_is_desktop: Option<bool>,
}

impl ScriptHook for HomeScreen {
    fn on_after_new(&mut self, vm: &mut ScriptVm) {
        self.reapply_view_mode(vm);
    }

    fn on_after_reload(&mut self, vm: &mut ScriptVm) {
        self.reapply_view_mode(vm);
    }
}

impl Widget for HomeScreen {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        if let Event::Scroll(scroll) = event
            && cx.is_scrolling_allowed_within(&self.view.area())
        {
            let back = self.back_swipe.update(scroll);
            let explore = scope.data.get::<AppState>().is_some_and(|app| app.selected_tab == SelectedTab::AddRoom);
            if back && (explore || !effective_is_desktop(cx)) && self.view.area().rect(cx).contains(scroll.abs) {
                self.handle_event(cx, &Event::BackPressed { handled: std::cell::Cell::new(false) }, scope);
                return;
            }
        }
        if let Event::Actions(actions) = event {
            // On desktop, the RoomFilterInputBar is inside this HomeScreen.
            // Check if it changed and re-emit as a MainFilterAction so that
            // RoomsList and SpacesBar can respond without cross-talk from
            // other RoomFilterInputBar instances (e.g., SpaceLobbyScreen's).
            if let Some(keywords) = self.view.room_filter_input_bar(cx, ids!(room_filter_input_bar)).changed(actions) {
                cx.action(MainFilterAction::Changed(keywords));
            }
            // The rooms-list header's search icon: on desktop the filter bar lives here.
            if actions.iter().any(|a| matches!(a.downcast_ref(), Some(RoomsListHeaderAction::OpenRoomFilterModal))) {
                let input = self.view.text_input(cx, ids!(room_filter_input_bar.input));
                if !input.is_empty() {
                    input.set_key_focus(cx);
                }
            }

            let app_state = scope.data.get_mut::<AppState>().unwrap();
            if !effective_is_desktop(cx) {
                let stack = self.view.stack_navigation(cx, ids!(view_stack));
                if !stack.is_transitioning() && self.mobile_chat_info.is_none() {
                    if let Some(parent) = stack.current_view() {
                        let view = stack.view_by_id(cx, parent);
                        if view.button(cx, ids!(chat_info_button)).clicked(actions) {
                            if let Some(SelectedRoom::JoinedRoom {room_name_id} | SelectedRoom::Thread {room_name_id, ..}) = app_state.selected_room.as_ref() {
                                if let Some((info_id, info)) = stack.create_view_from_template(cx, id!(ChatInfoStackNavigationView)) {
                                    info.mobile_chat_info(cx, ids!(chat_info)).show(cx, room_name_id.clone());
                                    stack.set_title(cx, info_id, crate::i18n::tr("Chat Info"));
                                    self.mobile_chat_info = Some(info_id);
                                    stack.push(cx, info_id);
                                }
                            }
                        }
                    }
                }
            }
            for action in actions {
                if let Some(super::room_history::RoomHistoryAction::Jump {room, event}) = action.downcast_ref() {
                    if !effective_is_desktop(cx) {
                        if self.mobile_chat_info.is_some() {
                            // Room search covers the main room, including when opened from a thread.
                            if matches!(app_state.selected_room.as_ref(), Some(SelectedRoom::Thread {..})) {
                                if let Some(previous) = app_state.selected_room.take() {previous.close_thread_timeline(cx);}
                                app_state.selected_room = Some(SelectedRoom::JoinedRoom {room_name_id: room.clone()});
                            }
                            self.pop_selected_screen_view(cx, app_state);
                        } else if !matches!(app_state.selected_room.as_ref(), Some(SelectedRoom::JoinedRoom {room_name_id}) if room_name_id.room_id() == room.room_id()) {
                            self.push_selected_screen_view(cx, app_state, SelectedRoom::JoinedRoom {room_name_id: room.clone()});
                        }
                        let stack = self.view.stack_navigation(cx, ids!(view_stack));
                        if let Some(view_id) = stack.destination_view().or_else(|| stack.current_view()) {
                            stack.view_by_id(cx, view_id).room_screen(cx, ids!(room_screen)).jump_to_history_event(cx, event.clone());
                        }
                    }
                }
                match action.downcast_ref() {
                    Some(NavigationBarAction::GoToTab(tab)) => {
                        self.switch_to_tab(cx, app_state, tab.clone());
                    }
                    Some(NavigationBarAction::GoToHome) => {
                        self.switch_to_tab(cx, app_state, SelectedTab::Home);
                    }
                    Some(NavigationBarAction::GoToAddRoom) => {
                        if app_state.selected_tab != SelectedTab::AddRoom {
                            self.explore_return_tab = if app_state.selected_tab == SelectedTab::Settings {
                                self.previous_selection.clone()
                            } else { app_state.selected_tab.clone() };
                        }
                        self.switch_to_tab(cx, app_state, SelectedTab::AddRoom);
                    }
                    Some(NavigationBarAction::GoToSpace { space_name_id }) => {
                        self.switch_to_tab(cx, app_state, SelectedTab::Space { space_name_id: space_name_id.clone() });
                    }
                    // Only open the settings screen if it is not currently open.
                    Some(NavigationBarAction::OpenSettings | NavigationBarAction::OpenOwnProfile) => {
                        if !matches!(app_state.selected_tab, SelectedTab::Settings) {
                            self.previous_selection = std::mem::replace(&mut app_state.selected_tab, SelectedTab::Settings);
                            cx.action(NavigationBarAction::TabSelected(app_state.selected_tab.clone()));
                            if let Some(settings_page) = self.update_active_page_from_selection(cx, app_state) {
                                settings_page
                                    .settings_screen(cx, ids!(settings_screen))
                                    .populate(cx, None, app_state);
                                if matches!(action.downcast_ref(), Some(NavigationBarAction::OpenOwnProfile)) {
                                    settings_page.settings_screen(cx, ids!(settings_screen)).open_personal_info(cx);
                                }
                                self.view.redraw(cx);
                            } else {
                                error!("BUG: failed to set active page to show settings screen.");
                            }
                        }
                    }
                    Some(NavigationBarAction::CloseSettings | NavigationBarAction::CloseAddRoom) => {
                        let expected = match action.downcast_ref() {
                            Some(NavigationBarAction::CloseAddRoom) => SelectedTab::AddRoom,
                            _ => SelectedTab::Settings,
                        };
                        if app_state.selected_tab == expected {
                            app_state.selected_tab = if expected == SelectedTab::AddRoom {
                                self.explore_return_tab.clone()
                            } else { self.previous_selection.clone() };
                            cx.action(NavigationBarAction::TabSelected(app_state.selected_tab.clone()));
                            self.update_active_page_from_selection(cx, app_state);
                            self.view.redraw(cx);
                        }
                    }
                    Some(NavigationBarAction::ToggleSpacesBar) => {
                        self.is_spaces_bar_shown = !self.is_spaces_bar_shown;
                        self.view.spaces_bar_wrapper(cx, ids!(spaces_bar_wrapper))
                            .show_or_hide(cx, self.is_spaces_bar_shown);
                    }
                    // We're the ones who emitted this action, so we don't need to handle it again.
                    Some(NavigationBarAction::TabSelected(_))
                    | None => { }
                }

                if let Some(super::mobile::MobileNavigationAction::DetailVisibility(open)) = action.downcast_ref() {
                    self.mobile_detail_open = *open;
                    self.view.redraw(cx);
                }

                // React to App Settings changes that affect the HomeScreen layout.
                if let Some(AppPreferencesAction::ViewModeChanged(new_mode)) = action.downcast_ref() {
                    if *new_mode != self.applied_view_mode {
                        self.apply_view_mode(cx, *new_mode);
                        // Set & broadcast the new variant now so that the mobile cleanup in
                        // `sync_effective_view_mode()` can run before the dock reloads.
                        if !matches!(new_mode, ViewModeOverride::Automatic)
                            || cx.display_context.is_screen_size_known()
                        {
                            // this dummy parent size is only read when the screen size is unknown
                            let variant = (new_mode.variant_selector())(cx, &Vec2d::default());
                            cx.global::<MainViewIsDesktop>().0 = Some(variant == live_id!(Desktop));
                        }
                        self.view.redraw(cx);
                    }
                }

                // An invited space's InviteScreen should be shown on the main home screen's dock,
                // so navigate to that first (un-select any selected space).
                //
                // This is to ensure that we don't show a new space invite within an existing
                // unrelated space's separate dock / rooms list.
                if let SpacesBarAction::InvitedSpaceClicked { space_name_id } = action.as_widget_action().cast()
                    && !effective_is_desktop(cx)
                {
                    self.switch_to_tab(cx, app_state, SelectedTab::Home);
                    self.push_selected_screen_view(
                        cx,
                        app_state,
                        SelectedRoom::InvitedRoom { room_name_id: space_name_id },
                    );
                    continue;
                }

                // Handle room selections. Desktop owns tab creation in MainDesktopUI,
                // while mobile owns StackNavigation screen pushes here.
                match action.as_widget_action().cast() {
                    RoomsListAction::Selected(selected_room) if !effective_is_desktop(cx) => {
                        self.push_selected_screen_view(cx, app_state, selected_room);
                    }
                    // On desktop, `MainDesktopUI` handles this, so we only need to update this in mobile view mode.
                    RoomsListAction::InviteAccepted { room_name_id, kind } if !effective_is_desktop(cx) => {
                        let is_space = kind.is_space();
                        self.upgrade_mobile_invite_to_joined(cx, app_state, &room_name_id, kind);
                        cx.action(AppStateAction::UpgradedInviteToJoinedRoom {
                            room_id: room_name_id.room_id().clone(),
                            is_space,
                        });
                    }
                    _ => {}
                }

                if let StackNavigationTransitionAction::ViewReleased(view_id) =
                    action.as_widget_action().cast()
                {
                    let stack_navigation = self.view.stack_navigation(cx, ids!(view_stack));
                    self.hide_screen_in_released_stack_view(cx, &stack_navigation, view_id);
                }

                // When a stack navigation pop is requested (back button pressed),
                // reveal the previous screen from HomeScreen's mobile history.
                if let StackNavigationAction::Pop = action.as_widget_action().cast() {
                    self.pop_selected_screen_view(cx, app_state);
                }

                if let Some(
                    AppStateAction::RoomNameUpdated(new_room_name)
                    | AppStateAction::RoomLoadedSuccessfully { room_name_id: new_room_name, .. }
                ) = action.downcast_ref() {
                    for room in &mut self.mobile_screen_history {
                        room.update_room_name(new_room_name);
                    }
                    self.previous_selection.update_space_name(new_room_name);
                    self.explore_return_tab.update_space_name(new_room_name);
                    let stack_navigation = self.view.stack_navigation(cx, ids!(view_stack));
                    if let Some(view_id) = stack_navigation.destination_view()
                        && let Some(room) = app_state.selected_room.as_ref()
                        && room.room_id() == new_room_name.room_id()
                    {
                        if self.mobile_chat_info != Some(view_id) {
                            stack_navigation.set_title(cx, view_id, &room.display_name());
                        }
                    }
                }
            }
        }

        // Consume a contact-page Back before the StackNavigation widget sees
        // its generated Pop on the next event turn (it otherwise pops to root).
        // Keep every unrelated widget/backend action in the same batch.
        let mut generated = cx.capture_actions(|cx| self.view.handle_event(cx, event, scope));
        generated.retain(|action| {
            !(matches!(action.as_widget_action().cast(), StackNavigationAction::Pop)
                && self.dismiss_mobile_profile(cx))
        });
        cx.extend_actions(generated);

        // Now that we've forwarded the event (above) to our children, the AdaptiveView instance
        // has properly updated its view mode, so we can now query and sync it across robrix.
        if let Event::Actions(_) = event {
            let app_state = scope.data.get_mut::<AppState>().unwrap();
            self.sync_effective_view_mode(cx, app_state);
        }
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let app_state = scope.data.get_mut::<AppState>().unwrap();
        // Note: We need to update the active page before drawing,
        // because if we switched between Desktop and Mobile views,
        // the PageFlip widget will have been reset to its default,
        // so we must re-set it to the correct page based on `app_state.selected_tab`.
        self.update_active_page_from_selection(cx, app_state);
        let show_tabs = matches!(app_state.selected_tab, SelectedTab::Home | SelectedTab::Contacts | SelectedTab::Discover | SelectedTab::Me | SelectedTab::Space {..})
            && !(app_state.selected_tab == SelectedTab::Contacts && self.mobile_detail_open);
        self.view.view(cx, ids!(mobile_navigation)).set_visible(cx, show_tabs);

        // Contact details share the stack's single mobile header. Query the
        // current view rather than retaining any released room widget.
        if !effective_is_desktop(cx) && self.mobile_chat_info.is_none()
            && matches!(app_state.selected_room, Some(SelectedRoom::JoinedRoom {..} | SelectedRoom::Thread {..}))
        {
            let stack = self.view.stack_navigation(cx, ids!(view_stack));
            if let Some(view_id) = stack.current_view() {
                let view = stack.view_by_id(cx, view_id);
                let profile_open = view.user_profile_sliding_pane(cx, ids!(user_profile_sliding_pane))
                    .is_currently_shown(cx);
                let title = if profile_open { crate::i18n::tr("Contact Info").to_owned() }
                    else { app_state.selected_room.as_ref().unwrap().display_name() };
                stack.set_title(cx, view_id, &title);
                view.view(cx, ids!(info_controls)).set_visible(cx, !profile_open);
            }
        }

        self.view.draw_walk(cx, scope, walk)
    }
}

impl HomeScreen {
    fn dismiss_mobile_profile(&mut self, cx: &mut Cx) -> bool {
        if effective_is_desktop(cx) || self.mobile_chat_info.is_some() { return false; }
        let stack = self.view.stack_navigation(cx, ids!(view_stack));
        let Some(view_id) = stack.current_view() else { return false; };
        let view = stack.view_by_id(cx, view_id);
        let profile = view.user_profile_sliding_pane(cx, ids!(user_profile_sliding_pane));
        if !profile.is_currently_shown(cx) { return false; }
        profile.dismiss(cx);
        self.view.redraw(cx);
        true
    }
    /// Installs a variant selector on the main `AdaptiveView` that honors the
    /// current [`ViewModeOverride`] preference, and publishes each choice so
    /// that `effective_is_desktop()` always matches that same view mode.
    fn apply_view_mode(&mut self, cx: &mut Cx, mode: ViewModeOverride) {
        let mut select_variant = mode.variant_selector();
        self.view
            .adaptive_view(cx, ids!(main_adaptive_view))
            .set_variant_selector(move |cx, parent_size| {
                let variant = select_variant(cx, parent_size);
                let is_desktop = variant == live_id!(Desktop);
                if cx.global::<MainViewIsDesktop>().0.replace(is_desktop) != Some(is_desktop) {
                    cx.action(MainViewVariantChangedAction);
                }
                variant
            });
        self.applied_view_mode = mode;
    }

    /// Reinstalls the AdaptiveView variant selector based on the current user preference.
    fn reapply_view_mode(&mut self, vm: &mut ScriptVm) {
        vm.with_cx_mut(|cx| {
            let mode = cx.global::<AppPreferencesGlobal>().0.view_mode;
            self.apply_view_mode(cx, mode);
        });
    }

    fn sync_effective_view_mode(&mut self, cx: &mut Cx, app_state: &mut AppState) {
        // Do nothing until the AdaptiveView instance has actually selected a variant
        let Some(is_desktop) = cx.global::<MainViewIsDesktop>().0 else { return };
        let Some(was_desktop) = self.last_effective_is_desktop.replace(is_desktop) else {
            return;
        };
        if was_desktop == is_desktop {
            return;
        }

        // If we transitioned from mobile --> desktop view mode, the dock will reload the tabs
        // from its previously-saved state, so we need to free the current selected room now
        // (if it was a thread timeline), and then also clear any thread timelines in the mobile nav stack.
        if !was_desktop && is_desktop {
            if let Some(room) = app_state.selected_room.as_ref() {
                room.close_thread_timeline(cx);
            }
        }

        // If the mentionable popup was shown, close it because the whole UI has changed/moved.
        if cx.has_global::<MentionablePopupRef>() {
            cx.get_global::<MentionablePopupRef>().clone().cancel(cx);
        }

        self.clear_mobile_navigation_state(cx);

        // Switching into mobile mode lands on the rooms list, so no room should
        // be drawn as selected until one is actually clicked.
        if !is_desktop {
            cx.action(AppStateAction::FocusNone);
        }
    }

    fn update_active_page_from_selection(
        &mut self,
        cx: &mut Cx,
        app_state: &mut AppState,
    ) -> Option<WidgetRef> {
        self.view
            .page_flip(cx, ids!(home_screen_page_flip))
            .set_active_page(
                cx,
                match app_state.selected_tab {
                    SelectedTab::Space { .. }
                    | SelectedTab::Home => id!(home_page),
                    SelectedTab::Contacts => id!(contacts_page),
                    SelectedTab::Discover => id!(discover_page),
                    SelectedTab::Me => id!(me_page),
                    SelectedTab::Settings => id!(settings_page),
                    SelectedTab::AddRoom => id!(add_room_page),
                },
            )
    }

    /// Populates a `StackNavigationView` with the given room/screen's info.
    ///
    /// Returns the LiveId of the view that should be pushed onto or revealed by
    /// the stack navigation widget.
    fn populate_mobile_stack_view(
        &mut self,
        cx: &mut Cx,
        stack_navigation: &StackNavigationRef,
        selected_screen: &SelectedRoom,
    ) -> Option<LiveId> {
        let view_id = match selected_screen {
            SelectedRoom::JoinedRoom { room_name_id }
            | SelectedRoom::Thread { room_name_id, .. } => {
                let Some((view_id, stack_navigation_view)) =
                    stack_navigation.create_view_from_template(cx, id!(RoomScreenStackNavigationView))
                else {
                    error!("BUG: failed to create mobile RoomScreen StackNavigationView");
                    return None;
                };
                Self::hide_displayed_stack_screen(cx, &stack_navigation_view);
                let thread_root = if let SelectedRoom::Thread { thread_root_event_id, .. } = selected_screen {
                    Some(thread_root_event_id.clone())
                } else {
                    None
                };
                stack_navigation_view
                    .room_screen(cx, ids!(room_screen))
                    .set_displayed_room(cx, room_name_id, thread_root);
                view_id
            }
            SelectedRoom::InvitedRoom { room_name_id } => {
                let Some((view_id, stack_navigation_view)) =
                    stack_navigation.create_view_from_template(cx, id!(InviteScreenStackNavigationView))
                else {
                    error!("BUG: failed to create mobile InviteScreen StackNavigationView");
                    return None;
                };
                Self::hide_displayed_stack_screen(cx, &stack_navigation_view);
                stack_navigation_view
                    .invite_screen(cx, ids!(invite_screen))
                    .set_displayed_invite(cx, room_name_id);
                view_id
            }
            SelectedRoom::Space { space_name_id } => {
                let Some((view_id, stack_navigation_view)) =
                    stack_navigation.create_view_from_template(cx, id!(SpaceLobbyScreenStackNavigationView))
                else {
                    error!("BUG: failed to create mobile SpaceLobbyScreen StackNavigationView");
                    return None;
                };
                Self::hide_displayed_stack_screen(cx, &stack_navigation_view);
                stack_navigation_view
                    .space_lobby_screen(cx, ids!(space_lobby_screen))
                    .set_displayed_space(cx, space_name_id);
                view_id
            }
        };

        stack_navigation.set_title(cx, view_id, &selected_screen.display_name());
        Some(view_id)
    }

    /// Hides the screen within a stack view that was released by the StackNavigation widget.
    fn hide_screen_in_released_stack_view(
        &mut self,
        cx: &mut Cx,
        stack_navigation: &StackNavigationRef,
        view_id: LiveId,
    ) {
        if stack_navigation.stack_view_ids().contains(&view_id) {
            return;
        }
        let stack_navigation_view = stack_navigation.view_by_id(cx, view_id);
        Self::hide_displayed_stack_screen(cx, &stack_navigation_view);
    }

    fn clear_mobile_navigation_state(&mut self, cx: &mut Cx) {
        self.mobile_chat_info = None;
        // When switching from mobile --> desktop view mode, we discard the nav stack,
        // and thus we need to free & destroy any thread timelines in it.
        // Note that freeing the current room is handled in `sync_effective_view_mode`.
        for room in &self.mobile_screen_history {
            room.close_thread_timeline(cx);
        }
        self.mobile_screen_history.clear();

        let stack_navigation = self.view.stack_navigation(cx, ids!(view_stack));
        for view_id in stack_navigation.dynamic_stack_view_ids() {
            let stack_navigation_view = stack_navigation.view_by_id(cx, view_id);
            Self::hide_displayed_stack_screen(cx, &stack_navigation_view);
        }
        // Also go back to the root stack view to ensure no old roomscreens persist.
        stack_navigation.pop_to_root(cx);
    }

    fn hide_displayed_stack_screen(cx: &mut Cx, stack_navigation_view: &WidgetRef) {
        stack_navigation_view.mobile_chat_info(cx, ids!(chat_info)).clear();
        stack_navigation_view
            .room_screen(cx, ids!(room_screen))
            .hide_displayed_room(cx);
        stack_navigation_view
            .invite_screen(cx, ids!(invite_screen))
            .hide_displayed_invite(cx);
        stack_navigation_view
            .space_lobby_screen(cx, ids!(space_lobby_screen))
            .hide_displayed_space(cx);
    }

    /// Pushes the given screen onto the mobile screen history and animates it in.
    fn push_selected_screen_view(
        &mut self,
        cx: &mut Cx,
        app_state: &mut AppState,
        sr: SelectedRoom,
    ) {
        let stack_navigation = self.view.stack_navigation(cx, ids!(view_stack));
        if stack_navigation.is_transitioning() {
            return;
        }
        let has_current_mobile_screen = stack_navigation.current_view().is_some();
        // If it has the same room ID and the same screen type (invite, joined, etc),
        // then we actually don't need to do anything. Otherwise we need to change it
        // to a new screen, e.g., a joined RoomScreen or a joined SpaceLobbyScreen.
        let is_same_screen = app_state.selected_room.as_ref().is_some_and(|c|
            c == &sr && std::mem::discriminant(c) == std::mem::discriminant(&sr)
        );
        if has_current_mobile_screen && is_same_screen {
            return;
        }
        let Some(view_id) = self.populate_mobile_stack_view(cx, &stack_navigation, &sr) else {
            return;
        };

        // Save the current selected_room onto the navigation stack before replacing it.
        if has_current_mobile_screen {
            if let Some(prev) = app_state.selected_room.take() {
                self.mobile_screen_history.push(prev);
            }
        }
        app_state.selected_room = Some(sr);
        stack_navigation.push(cx, view_id);
        self.view.redraw(cx);
    }

    /// Switches to (selects) the given navigation tab, if it isn't already the selected one.
    fn switch_to_tab(&mut self, cx: &mut Cx, app_state: &mut AppState, new_tab: SelectedTab) {
        if app_state.selected_tab == new_tab { return }
        self.previous_selection = std::mem::replace(&mut app_state.selected_tab, new_tab);
        cx.action(NavigationBarAction::TabSelected(app_state.selected_tab.clone()));
        self.update_active_page_from_selection(cx, app_state);
        self.view.redraw(cx);
    }

    /// Upgrades a room's or space's InviteScreen that is being shown in mobile view mode
    /// to that newly-joined RoomScreen or SpaceLobbyScreen.
    fn upgrade_mobile_invite_to_joined(
        &mut self,
        cx: &mut Cx,
        app_state: &mut AppState,
        room_name_id: &RoomNameId,
        kind: AcceptedInviteKind,
    ) {
        let is_space = kind.is_space();
        let room_id = room_name_id.room_id();
        let is_this_invite = |sr: &SelectedRoom| matches!(
            sr, SelectedRoom::InvitedRoom { room_name_id: r } if r.room_id() == room_id
        );
        if app_state.selected_room.as_ref().is_some_and(is_this_invite) {
            // The new SpaceLobbyScreen should be shown within its top-level ancestor space's tab/dock,
            // so navigate to that, which also sets the rooms list into that space's mode.
            if let AcceptedInviteKind::Space { dock_space: Some(dock_space) } = &kind {
                self.switch_to_tab(cx, app_state, SelectedTab::Space { space_name_id: dock_space.clone() });
            }
            self.push_selected_screen_view(
                cx,
                app_state,
                SelectedRoom::to_joined(room_name_id.clone(), is_space),
            );
            // The invite we replaced was pushed onto the mobile history stack,
            // so we need to remove it to ensure that it won't show up if the user goes back.
            if self.mobile_screen_history.last().is_some_and(is_this_invite) {
                self.mobile_screen_history.pop();
            }
        }
        for room in &mut self.mobile_screen_history {
            room.upgrade_invite_to_joined(room_id, is_space);
        }
    }

    /// Pops the current mobile screen, revealing the previous screen or the room list root.
    fn pop_selected_screen_view(&mut self, cx: &mut Cx, app_state: &mut AppState) {
        let stack_nav = self.view.stack_navigation(cx, ids!(view_stack));
        if stack_nav.is_transitioning() {
            return;
        }
        if self.dismiss_mobile_profile(cx) { return; }
        if self.mobile_chat_info.is_some() {
            // StackNavigation releases the covered room. Rehydrate it through
            // the regular timeline/draft restore path before revealing it.
            // Keeping a released room live retains stale cached draw areas.
            if let Some(selected) = app_state.selected_room.as_ref() {
                if let Some(view_id) = self.populate_mobile_stack_view(cx, &stack_nav, selected) {
                    self.mobile_chat_info = None;
                    stack_nav.pop_to_view(cx, view_id);
                    self.view.redraw(cx);
                }
            }
            return;
        }
        let Some(current_screen) = app_state.selected_room.take() else {
            // If we didn't have a current screen, something's buggy,
            // so the safest option is to clear the mobile stack and start over. nbd.
            self.mobile_screen_history.clear();
            return;
        };
        match self.mobile_screen_history.pop() {
            Some(previous) => {
                let Some(view_id) = self.populate_mobile_stack_view(cx, &stack_nav, &previous) else {
                    // Nav failed; current_screen is restored, so don't free it.
                    app_state.selected_room = Some(current_screen);
                    self.mobile_screen_history.push(previous);
                    return;
                };
                // current_screen is gone for good — free its thread timeline if it is one.
                current_screen.close_thread_timeline(cx);
                app_state.selected_room = Some(previous);
                stack_nav.pop_to_view(cx, view_id);
            }
            None => {
                current_screen.close_thread_timeline(cx);
                app_state.selected_room = None;
                stack_nav.pop_to_root(cx);
                log!("Rinx navigation: returned to chat list");
            }
        }
        self.view.redraw(cx);
    }
}
