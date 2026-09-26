//! Native Moments surfaces; content comes from the active Matrix account.
use std::{collections::{BTreeSet, HashMap}, path::PathBuf};
use makepad_widgets::*;
use ruma::{OwnedUserId, OwnedRoomId, OwnedEventId, TransactionId};
use crate::{
    sliding_sync::{current_user_id, spawn_async_task},
    media_cache::{MediaCache, MediaCacheEntry},
    shared::{
        navigation_bar_button::NavigationBarButtonAction,
        text_or_image::{TextOrImageAction, TextOrImageWidgetRefExt, TextOrImageWidgetExt},
        avatar::AvatarWidgetRefExt,
        attachment_download::{
            DownloadableAttachment, DownloadKind, media_source_mxc, start_attachment_download,
        },
    },
    home::back_swipe::BackSwipe,
};
use super::{
    backend::{Service, Feed, Timeline, Pending, ComposerDraft, ReplyDraftKey},
    model::{Entry, Asset, MAX_MEDIA},
};

#[derive(Clone, Debug)]
pub enum MomentsAction {
    Open { author: Option<OwnedUserId> },
    Compose { text: String },
    FileTransfer,
    Close,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Page {
    #[default]
    Feed,
    Compose,
    Details,
    Audience,
    Invitations,
    Transfer,
}
#[derive(Clone, Debug)]
enum Command {
    Refresh {
        older: bool,
        origin: RefreshOrigin,
    },
    Prepare,
    RetrySetup,
    Audience,
    Send(Pending),
    Retry,
    Discard,
    Review(OwnedRoomId, String),
    Comment(Entry, String),
    Edit(Entry, String),
    Like(Entry),
    Redact(OwnedRoomId, Vec<OwnedEventId>),
    Invite(OwnedRoomId, OwnedUserId),
    Remove(OwnedRoomId, OwnedUserId),
    Invitation(OwnedRoomId, bool),
    Choose(OwnedRoomId),
    Hide(OwnedUserId, bool),
    Seen(Vec<OwnedEventId>),
    FileTransfer(bool),
    ShareWithDmContacts(bool),
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum RefreshOrigin {
    Initial,
    Automatic,
    Manual,
    FollowUp,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum CommandFeedback {
    Default,
    Silent,
    Refresh(RefreshOrigin),
}
impl Command {
    fn feedback(&self) -> CommandFeedback {
        match self {
            Self::Refresh { origin, .. } => CommandFeedback::Refresh(*origin),
            Self::Seen(_) => CommandFeedback::Silent,
            _ => CommandFeedback::Default,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum RefreshFailureNotice {
    None,
    InlineUnavailable,
    ImmediateWarning,
    StaleWarning,
}
const BACKGROUND_REFRESH_WARNING_THRESHOLD: u8 = 3;

fn refresh_failure_notice(
    origin: RefreshOrigin,
    has_loaded_feed: bool,
    consecutive_failures: u8,
    warning_shown: bool,
) -> RefreshFailureNotice {
    if !has_loaded_feed {
        return RefreshFailureNotice::InlineUnavailable;
    }
    if origin == RefreshOrigin::Manual {
        return RefreshFailureNotice::ImmediateWarning;
    }
    if matches!(origin, RefreshOrigin::Automatic | RefreshOrigin::FollowUp)
        && consecutive_failures >= BACKGROUND_REFRESH_WARNING_THRESHOLD
        && !warning_shown
    {
        return RefreshFailureNotice::StaleWarning;
    }
    RefreshFailureNotice::None
}
#[derive(Clone, Debug)]
enum Outcome {
    Feed(Feed),
    Ready(Timeline),
    Changed,
    Sent(SentKind),
    Transfer(OwnedRoomId),
}
#[derive(Clone, Debug)]
enum SentKind {
    Post { body: String, paths: Vec<PathBuf> },
    Reply { key: ReplyDraftKey, body: String },
    Edit(OwnedEventId),
    Other,
}
#[derive(Clone, Debug)]
struct Completed {
    owner: OwnedUserId,
    request: u64,
    feedback: CommandFeedback,
    result: Result<Outcome, String>,
}
#[derive(Clone, Debug)]
struct Picked {
    owner: OwnedUserId,
    session: u64,
    result: Result<PathBuf, String>,
}
/// The gap between photos in a Moments photo grid.
const ALBUM_GAP: f64 = 3.0;
const CAPTION: f64 = if cfg!(target_os = "macos") { 28.0 } else { 0.0 };

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    let ActionButton = RobrixNeutralIconButton {width: Fill height: 40 spacing: 0 icon_walk: Walk{width: 0 height: 0}
        draw_bg +: {color: #x00000000 color_hover: #xe4e4e4 color_down: #xd0d0d0}
    }
    let Hint = Label {width: Fill height: Fit flow: Flow.Right{wrap: true} draw_text +: {color: #x888888 text_style: theme.font_regular{font_size: 10}}}
    let Body = Label {width: Fill height: Fit flow: Flow.Right{wrap: true} draw_text +: {color: #x191919 text_style: theme.font_regular{font_size: 12}}}
    // A photo cropped to fill its square grid cell, like WeChat Moments.
    let Photo = TextOrImage {width: Fill height: Fill
        image_view +: {height: Fill image +: {width: Fill height: Fill fit: ImageFit.CropToFill}}
        text_view +: {height: Fill label +: {max_lines: 2 draw_text.text_style.font_size: 9}}
    }
    // Keeps its third of the row even when its photo is hidden, so a partly
    // filled row doesn't stretch its photos.
    let Cell = View {width: Fill height: Fill}
    mod.widgets.MomentsPanel = #(MomentsPanel::register_widget(vm)) {
        ..mod.widgets.SolidView
        width: Fill height: Fill flow: Down draw_bg.color: #xededed
        padding: Inset{top: SAFE_INSET_PAD_TOP + #(CAPTION) bottom: SAFE_INSET_PAD_BOTTOM}
        header := DetailHeader {title.text: #(crate::i18n::tr("Moments")) title.i18n_text: "Moments"}
        moments_status := Hint {margin: Inset{left: 16 right: 16 top: 6 bottom: 6}}
        feed_page := View {width: Fill height: Fill flow: Down
            View {width: Fill height: 40 flow: Right padding: Inset{left: 10 right: 10}
                moments_refresh := ActionButton {text: #(crate::i18n::tr("Refresh")) i18n_text: "Refresh"}
                moments_invites := ActionButton {text: #(crate::i18n::tr("Invitations")) i18n_text: "Invitations"}
                moments_audience := ActionButton {text: #(crate::i18n::tr("Audience")) i18n_text: "Audience"}
                moments_compose := RobrixPositiveIconButton {text: #(crate::i18n::tr("Post")) i18n_text: "Post" width: 60 height: 36 spacing: 0 icon_walk: Walk{width: 0 height: 0}}
            }
            moments_feed := PortalList {width: Fill height: Fill
                Cover := SolidView {width: Fill height: 158 flow: Down padding: 22 align: Align{y: 1.0} spacing: 8
                    draw_bg.color: #x344d43
                    cover_name := Label {width: Fill flow: Flow.Right{wrap: true} draw_text +: {color: #xffffff text_style: theme.font_bold{font_size: 19}}}
                    Label {text: #(crate::i18n::tr("Small moments, shared with friends")) i18n_text: "Small moments, shared with friends" draw_text +: {color: #xc5d9cd text_style: theme.font_regular{font_size: 10}}}
                }
                Post := NavigationBarButton {width: Fill height: Fit flow: Right padding: 16 spacing: 12 align: Align{x: 0.0 y: 0.0}
                    draw_bg +: {color_hover: #xf4f4f4 border_radius: 0 get_color: fn() -> vec4{return #xffffff.mix(self.color_hover,self.hover)}}
                    post_avatar := MobileAvatar {width: 38 height: 38}
                    View {width: Fill height: Fit flow: Down spacing: 10
                    post_author := Label {width: Fill max_lines: 1 text_overflow: Ellipsis draw_text +: {color: #x576b95 text_style: theme.font_bold{font_size: 12}}}
                    post_body := Body {max_lines: 6 text_overflow: Ellipsis}
                    // Rows are made square-celled at draw time; see `square_album_rows()`.
                    // The transparent backgrounds give the grid and rows a measurable area.
                    album := View {width: Fill height: Fit flow: Down spacing: 3 visible: false show_bg: true draw_bg.color: #x00000000
                        row0 := View {width: Fill height: 82 flow: Right spacing: 3 show_bg: true draw_bg.color: #x00000000 Cell{a0 := Photo{}} Cell{a1 := Photo{}} Cell{a2 := Photo{}}}
                        row1 := View {width: Fill height: 82 flow: Right spacing: 3 show_bg: true draw_bg.color: #x00000000 Cell{a3 := Photo{}} Cell{a4 := Photo{}} Cell{a5 := Photo{}}}
                        row2 := View {width: Fill height: 82 flow: Right spacing: 3 show_bg: true draw_bg.color: #x00000000 Cell{a6 := Photo{}} Cell{a7 := Photo{}} Cell{a8 := Photo{}}}
                    }
                    post_meta := Hint {}
                    post_interactions := Hint {draw_text.color: #x576b95}
                    SolidView {width: Fill height: 0.5 draw_bg.color: #xe5e5e5}
                    }
                }
                Filler := SolidView {width: Fill height: 100 draw_bg.color: #xffffff}
                Empty := View {width: Fill height: Fit padding: 24
                    empty_text := Body {text: #(crate::i18n::tr("No posts yet. Post your first moment, or accept a friend's timeline invitation.")) i18n_text: "No posts yet. Post your first moment, or accept a friend's timeline invitation."}
                }
            }
            moments_more := ActionButton {text: #(crate::i18n::tr("Load older / more timelines")) i18n_text: "Load older / more timelines"}
        }
        compose_page := ScrollYView {visible: false width: Fill height: Fill flow: Down padding: 16 spacing: 14
            composer_audience := Body {draw_text.color: #x576b95}
            Hint {text: #(crate::i18n::tr("Everyone in this timeline can see its posts, comments, likes and members. Invitations apply to this whole timeline. Earlier history may be unavailable to new viewers.")) i18n_text: "Everyone in this timeline can see its posts, comments, likes and members. Invitations apply to this whole timeline. Earlier history may be unavailable to new viewers."}
            moments_body := TextInput {width: Fill height: 150 empty_text: #(crate::i18n::tr("What's on your mind?")) i18n_empty_text: "What's on your mind?" is_multiline: true}
            // Thumbnails of the photos/videos picked for this post, in a 3x3 grid like WeChat.
            compose_album := View {width: Fill height: Fit flow: Down spacing: 3 visible: false show_bg: true draw_bg.color: #x00000000
                row0 := View {width: Fill height: 96 flow: Right spacing: 3 show_bg: true draw_bg.color: #x00000000 Cell{c0 := Photo{}} Cell{c1 := Photo{}} Cell{c2 := Photo{}}}
                row1 := View {width: Fill height: 96 flow: Right spacing: 3 show_bg: true draw_bg.color: #x00000000 Cell{c3 := Photo{}} Cell{c4 := Photo{}} Cell{c5 := Photo{}}}
                row2 := View {width: Fill height: 96 flow: Right spacing: 3 show_bg: true draw_bg.color: #x00000000 Cell{c6 := Photo{}} Cell{c7 := Photo{}} Cell{c8 := Photo{}}}
            }
            selected_media := Hint {}
            View {width: Fill height: 40 flow: Right spacing: 8
                moments_add_media := ActionButton {text: #(crate::i18n::tr("Add photo / video")) i18n_text: "Add photo / video"}
                moments_clear_media := ActionButton {text: #(crate::i18n::tr("Clear media")) i18n_text: "Clear media"}
            }
            compose_audience := ActionButton {text: #(crate::i18n::tr("Review Timeline Audience")) i18n_text: "Review Timeline Audience"}
            moments_publish := RobrixPositiveIconButton {text: #(crate::i18n::tr("Post")) i18n_text: "Post" width: Fill height: 44 spacing: 0 icon_walk: Walk{width: 0 height: 0}}
            moments_retry := ActionButton {text: #(crate::i18n::tr("Retry saved post / comment")) i18n_text: "Retry saved post / comment" visible: false}
            moments_discard := ActionButton {text: #(crate::i18n::tr("Discard saved retry")) i18n_text: "Discard saved retry" visible: false}
            retry_hint := Hint {visible: false text: #(crate::i18n::tr("Discarding stops retries. It cannot unsend content already delivered.")) i18n_text: "Discarding stops retries. It cannot unsend content already delivered."}
            Hint {text: #(crate::i18n::tr("Posts and comments are encrypted. Standard Matrix likes expose reaction metadata. Files are limited to nine items, 25 MB each.")) i18n_text: "Posts and comments are encrypted. Standard Matrix likes expose reaction metadata. Files are limited to nine items, 25 MB each."}
        }
        details_page := View {visible: false width: Fill height: Fill flow: Down
            details_scroll := ScrollYView {width: Fill height: Fill flow: Down padding: 16 spacing: 14
                detail_author := Body {draw_text.color: #x576b95}
                detail_body := Body {}
                detail_media := TextOrImage {width: Fill height: 260 visible: false image_view +: {height: Fill image +: {height: Fill fit: ImageFit.Smallest}}}
                media_controls := View {width: Fill height: 40 flow: Right spacing: 8 visible: false
                    media_previous := ActionButton {text: #(crate::i18n::tr("Previous")) i18n_text: "Previous"}
                    media_download := ActionButton {text: #(crate::i18n::tr("Download")) i18n_text: "Download"}
                    media_next := ActionButton {text: #(crate::i18n::tr("Next")) i18n_text: "Next"}
                }
                detail_meta := Hint {}
                detail_likes := Body {draw_text.color: #x576b95}
                View {width: Fill height: 40 flow: Right spacing: 8
                    moments_like := ActionButton {text: #(crate::i18n::tr("Like")) i18n_text: "Like"}
                    moments_hide := ActionButton {text: #(crate::i18n::tr("Hide author")) i18n_text: "Hide author"}
                }
                owner_actions := View {width: Fill height: 40 flow: Right spacing: 8
                    moments_edit := ActionButton {text: #(crate::i18n::tr("Edit post")) i18n_text: "Edit post"}
                    moments_delete := ActionButton {text: #(crate::i18n::tr("Delete post")) i18n_text: "Delete post" draw_text.color: #xfa5151}
                }
                Body {text: #(crate::i18n::tr("Comments")) i18n_text: "Comments"}
                comments := PortalList {width: Fill height: 220
                    Comment := View {width: Fill height: Fit flow: Down padding: Inset{top: 8 bottom: 8} spacing: 4
                        comment_name := Hint {draw_text.color: #x576b95}
                        comment_body := Body {}
                        comment_actions := View {width: Fill height: 32 flow: Right
                            comment_edit := ActionButton {text: #(crate::i18n::tr("Edit")) i18n_text: "Edit" height: 32}
                            comment_delete := ActionButton {text: #(crate::i18n::tr("Delete")) i18n_text: "Delete" height: 32}
                        }
                    }
                }
            }
            editor_hint := Hint {visible: false margin: Inset{left: 16 right: 16}}
            View {width: Fill height: 54 flow: Right padding: 8 spacing: 8
                moments_comment := TextInput {width: Fill height: Fill empty_text: #(crate::i18n::tr("Comment")) i18n_empty_text: "Comment"}
                moments_comment_send := RobrixPositiveIconButton {text: #(crate::i18n::tr("Send")) i18n_text: "Send" width: 60 height: Fill spacing: 0 icon_walk: Walk{width: 0 height: 0}}
            }
        }
        audience_page := View {visible: false width: Fill height: Fill flow: Down padding: 16 spacing: 12
            // Share with DM contacts, like WeChat's friend circle; see `super::dm_sharing`.
            View {width: Fill height: Fit flow: Right spacing: 12 align: Align{y: 0.5}
                Body {width: Fill text: #(crate::i18n::tr("Share with everyone I chat with 1-on-1")) i18n_text: "Share with everyone I chat with 1-on-1"}
                share_dm_contacts := ToggleFlat {
                    width: 46 height: 28 padding: 0 text: "" label_walk: Walk{width: 0 height: 0}
                    draw_bg +: {pixel: fn() {
                        let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                        sdf.box(0.0, 0.0, 46.0, 28.0, 14.0)
                        sdf.fill((#xdcdcdc).mix(#x07c160, self.active))
                        sdf.circle(14.0 + 18.0 * self.active, 14.0, 12.0)
                        sdf.fill(#xffffff)
                        return sdf.result
                    }}
                }
            }
            Hint {text: #(crate::i18n::tr("DM contacts who use Rinx and share this way join your audience automatically, and you join theirs. On by default; your Matrix profile shows that you share this way.")) i18n_text: "DM contacts who use Rinx and share this way join your audience automatically, and you join theirs. On by default; your Matrix profile shows that you share this way."}
            Hint {text: #(crate::i18n::tr("One audience for all your posts. Viewers see each other's comments, likes and membership. Removing a viewer prevents future access after sync; it cannot recall content already received.")) i18n_text: "One audience for all your posts. Viewers see each other's comments, likes and membership. Removing a viewer prevents future access after sync; it cannot recall content already received."}
            audience_name := Body {draw_text.color: #x576b95}
            setup_recovery := View {visible: false width: Fill height: Fit flow: Down spacing: 6
                Hint {text: #(crate::i18n::tr("If earlier setup never completed, retry after reconnecting. Any duplicate timelines stay separate; their audiences will never be merged.")) i18n_text: "If earlier setup never completed, retry after reconnecting. Any duplicate timelines stay separate; their audiences will never be merged."}
                moments_retry_setup := ActionButton {text: #(crate::i18n::tr("Retry timeline setup")) i18n_text: "Retry timeline setup"}
            }
            audience_list := PortalList {width: Fill height: Fill
                Member := View {width: Fill height: Fit flow: Down padding: Inset{top: 8 bottom: 8} spacing: 5
                    member_name := Body {} member_id := Hint {}
                    remove_viewer := ActionButton {text: #(crate::i18n::tr("Remove viewer")) i18n_text: "Remove viewer" height: 32 draw_text.color: #xfa5151}
                }
                Choice := View {width: Fill height: Fit flow: Down spacing: 6 padding: 8
                    choice_name := Body {} choose_timeline := ActionButton {text: #(crate::i18n::tr("Use this timeline")) i18n_text: "Use this timeline"}
                }
                Hidden := View {width: Fill height: Fit flow: Down spacing: 6 padding: 8
                    hidden_name := Body {} unhide_author := ActionButton {text: #(crate::i18n::tr("Show author in feed")) i18n_text: "Show author in feed"}
                }
            }
            audience_invite_controls := View {width: Fill height: 40 flow: Right spacing: 8
                audience_user := TextInput {width: Fill height: Fill empty_text: #(crate::i18n::tr("@friend:homeserver")) i18n_empty_text: "@friend:homeserver" autocapitalize: None}
                audience_invite := ActionButton {text: #(crate::i18n::tr("Invite")) i18n_text: "Invite" width: 65}
            }
            audience_review := ActionButton {text: #(crate::i18n::tr("Confirm audience for saved retry")) i18n_text: "Confirm audience for saved retry"}
        }
        invitations_page := View {visible: false width: Fill height: Fill flow: Down padding: 16 spacing: 12
            Hint {text: #(crate::i18n::tr("Join only if you want to read this person's Moments. Other viewers can see your membership, comments and likes. This does not grant them access to your own posts.")) i18n_text: "Join only if you want to read this person's Moments. Other viewers can see your membership, comments and likes. This does not grant them access to your own posts."}
            invitation_list := PortalList {width: Fill height: Fill
                Invitation := View {width: Fill height: Fit flow: Down padding: 10 spacing: 10
                    invitation_author := Body {}
                    View {width: Fill height: 40 flow: Right spacing: 8
                        accept_moments := ActionButton {text: #(crate::i18n::tr("Join timeline")) i18n_text: "Join timeline"}
                        reject_moments := ActionButton {text: #(crate::i18n::tr("Decline")) i18n_text: "Decline"}
                    }
                }
            }
            invitation_empty := Body {text: #(crate::i18n::tr("No timeline invitations.")) i18n_text: "No timeline invitations."}
        }
        transfer_page := View {visible: false width: Fill height: Fill flow: Down padding: 20 spacing: 16
            Body {text: #(crate::i18n::tr("File Transfer is a private chat for your own devices. It is separate from Moments.")) i18n_text: "File Transfer is a private chat for your own devices. It is separate from Moments."}
            transfer_retry := ActionButton {text: #(crate::i18n::tr("Open File Transfer")) i18n_text: "Open File Transfer"}
            transfer_new := ActionButton {text: #(crate::i18n::tr("New Private File Transfer")) i18n_text: "New Private File Transfer"}
        }
    }
}

#[derive(Script, ScriptHook, Widget)]
pub struct MomentsPanel {
    #[source]
    source: ScriptObjectRef,
    #[deref]
    view: View,
    #[rust]
    owner: Option<OwnedUserId>,
    #[rust]
    author: Option<OwnedUserId>,
    #[rust]
    feed: Feed,
    #[rust]
    posts: Vec<Entry>,
    #[rust]
    page: Page,
    #[rust]
    audience_return: Page,
    #[rust]
    timeline: Option<Timeline>,
    #[rust]
    detail: Option<Entry>,
    #[rust]
    editing: Option<Entry>,
    #[rust]
    comments: Vec<Entry>,
    #[rust]
    paths: Vec<PathBuf>,
    /// The `paths` currently loaded into the composer's thumbnail grid.
    #[rust]
    album_paths: Vec<PathBuf>,
    /// Square cell sides for the feed's and the composer's photo grids, as last measured.
    #[rust]
    feed_album_side: f64,
    #[rust]
    compose_album_side: f64,
    /// The height last applied to each photo-grid row, to avoid re-applying it every frame.
    #[rust]
    album_row_heights: HashMap<WidgetUid, f64>,
    #[rust]
    media: Option<MediaCache>,
    #[rust]
    media_index: usize,
    #[rust]
    busy: bool,
    #[rust]
    mutating: bool,
    #[rust]
    timer: Timer,
    #[rust]
    pending: Option<Pending>,
    #[rust]
    in_flight: Option<SentKind>,
    #[rust]
    request: u64,
    #[rust]
    session: u64,
    #[rust]
    status: String,
    #[rust]
    refresh_notice: String,
    #[rust]
    has_loaded_feed: bool,
    #[rust]
    consecutive_refresh_failures: u8,
    #[rust]
    refresh_warning_shown: bool,
    #[rust]
    back_swipe: BackSwipe,
}
fn next_request() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
fn post_draft_matches(body: &str, paths: &[PathBuf], draft_body: &str, draft_paths: &[PathBuf]) -> bool {
    paths == draft_paths && body == draft_body
}
fn sent_kind_for_pending(pending: &Pending) -> SentKind {
    if pending.is_post {
        SentKind::Post {
            body: pending.draft_body.clone().unwrap_or_else(||
                pending.content["body"].as_str().unwrap_or("").to_owned()),
            paths: pending.paths.clone(),
        }
    } else if let Some((key, body)) = super::backend::pending_reply_draft(pending) {
        SentKind::Reply { key, body }
    } else {
        SentKind::Other
    }
}

fn composer_draft_from_bytes(bytes: Option<&[u8]>) -> ComposerDraft {
    bytes
        .and_then(|bytes| serde_json::from_slice(bytes).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod draft_tests {
    use super::*;

    #[test]
    fn switching_to_account_without_draft_clears_previous_composer() {
        let first = ComposerDraft {
            body: "private note".into(),
            paths: vec![PathBuf::from("private-photo.png")],
        };
        let first_bytes = serde_json::to_vec(&first).unwrap();
        assert_eq!(composer_draft_from_bytes(Some(&first_bytes)).body, first.body);

        let second = composer_draft_from_bytes(None);
        assert!(second.body.is_empty());
        assert!(second.paths.is_empty());
    }

    #[test]
    fn corrupt_draft_does_not_retain_previous_composer() {
        let draft = composer_draft_from_bytes(Some(b"not json"));
        assert!(draft.body.is_empty());
        assert!(draft.paths.is_empty());
    }
}
impl MomentsPanel {
    fn detail_key(&self) -> Option<ReplyDraftKey> {
        self.detail.as_ref().map(|post| ReplyDraftKey {
            room: post.room.clone(), post: post.id.clone(),
        })
    }
    fn save_current_reply(&self, cx: &mut Cx) {
        if self.editing.is_some() { return; }
        let (Some(owner), Some(key)) = (&self.owner, self.detail_key()) else { return; };
        let body = self.text_input(cx, ids!(moments_comment)).text();
        if matches!(&self.in_flight, Some(SentKind::Reply { key: sending_key, body: sending_body })
            if sending_key == &key && sending_body == &body) { return; }
        if let Err(e) = super::backend::save_reply_draft(owner, &key, &body) {
            crate::shared::popup_list::enqueue_popup_notification(
                crate::i18n::format("Could not save Moments draft: {e}", &[("e", e.to_string())]),
                crate::shared::popup_list::PopupKind::Error, Some(5.0));
        }
    }
    fn restore_current_reply(&mut self, cx: &mut Cx) {
        let body = match (&self.owner, self.detail_key()) {
            (Some(owner), Some(key)) => match super::backend::reply_draft(owner, &key) {
                Ok(body) => body,
                Err(e) => {
                    self.status = crate::i18n::format("Could not load Moments draft: {e}", &[("e", e.to_string())]);
                    String::new()
                }
            },
            _ => String::new(),
        };
        self.text_input(cx, ids!(moments_comment)).set_text(cx, &body);
    }
    fn leave_detail(&mut self, cx: &mut Cx) {
        self.save_current_reply(cx);
        self.editing = None;
        self.detail = None;
        self.text_input(cx, ids!(moments_comment)).set_text(cx, "");
    }
    fn start_edit(&mut self, cx: &mut Cx, entry: Entry) {
        self.save_current_reply(cx);
        self.text_input(cx, ids!(moments_comment)).set_text(cx, entry.body());
        self.editing = Some(entry);
    }
    fn save_draft(&self, cx: &mut Cx) {
        let Some(owner) = &self.owner else { return };
        let body = self.text_input(cx, ids!(moments_body)).text();
        if matches!(&self.in_flight, Some(SentKind::Post { body: sending_body, paths })
            if post_draft_matches(sending_body, paths, &body, &self.paths)) { return; }
        let path = crate::persistence::persistent_state_dir(owner).join("moments-composer.json");
        if body.is_empty() && self.paths.is_empty() {
            let _ = std::fs::remove_file(path);
            return;
        }
        if let Err(e) = super::backend::write_private(
            &path,
            &ComposerDraft {
                body,
                paths: self.paths.clone(),
            },
        ) {
            crate::shared::popup_list::enqueue_popup_notification(
                crate::i18n::format("Could not save Moments draft: {e}", &[("e", (e).to_string())]),
                crate::shared::popup_list::PopupKind::Error,
                Some(5.0),
            );
        }
    }
    fn restore_draft(&mut self, cx: &mut Cx) {
        let Some(owner) = &self.owner else { return };
        let path = crate::persistence::persistent_state_dir(owner).join("moments-composer.json");
        let bytes = std::fs::read(path).ok();
        let draft = composer_draft_from_bytes(bytes.as_deref());
        self.text_input(cx, ids!(moments_body)).set_text(cx, &draft.body);
        self.paths = draft.paths;
    }
    fn reset(&mut self, cx: &mut Cx) {
        self.save_draft(cx);
        if self.page == Page::Details { self.save_current_reply(cx); }
        self.text_input(cx, ids!(moments_body)).set_text(cx, "");
        self.paths.clear();
        self.album_paths.clear();
        cx.stop_timer(self.timer);
        self.pending = None;
        self.in_flight = None;
        self.owner = None;
        self.author = None;
        self.feed = Feed::default();
        self.posts.clear();
        self.timeline = None;
        self.detail = None;
        self.editing = None;
        self.comments.clear();
        self.paths.clear();
        self.media = None;
        self.page = Page::Feed;
        self.busy = false;
        self.request = next_request();
        self.session = next_request();
        self.status.clear();
        self.refresh_notice.clear();
        self.has_loaded_feed = false;
        self.consecutive_refresh_failures = 0;
        self.refresh_warning_shown = false;
        self.text_input(cx, ids!(moments_body)).set_text(cx, "");
        self.text_input(cx, ids!(moments_comment)).set_text(cx, "");
    }
    fn refresh_succeeded(&mut self, origin: RefreshOrigin) {
        self.has_loaded_feed = true;
        self.consecutive_refresh_failures = 0;
        self.refresh_warning_shown = false;
        self.refresh_notice.clear();
        if origin != RefreshOrigin::Automatic {
            self.status.clear();
        }
    }
    fn refresh_failed(&mut self, origin: RefreshOrigin, error: &str) {
        log!("Moments refresh failed ({origin:?}): {error}");
        self.consecutive_refresh_failures = self.consecutive_refresh_failures.saturating_add(1);
        match refresh_failure_notice(
            origin,
            self.has_loaded_feed,
            self.consecutive_refresh_failures,
            self.refresh_warning_shown,
        ) {
            RefreshFailureNotice::None => {}
            RefreshFailureNotice::InlineUnavailable => {
                self.refresh_notice = crate::i18n::tr(
                    "Moments couldn't refresh. Check the service or network, then retry.",
                )
                .into();
            }
            RefreshFailureNotice::ImmediateWarning => {
                crate::shared::popup_list::enqueue_popup_notification(
                    crate::i18n::tr(
                        "Moments couldn't refresh. Check the service or network, then retry.",
                    ),
                    crate::shared::popup_list::PopupKind::Warning,
                    Some(6.0),
                );
            }
            RefreshFailureNotice::StaleWarning => {
                self.refresh_warning_shown = true;
                crate::shared::popup_list::enqueue_popup_notification(
                    crate::i18n::tr(
                        "Moments may be out of date. Check the service or network.",
                    ),
                    crate::shared::popup_list::PopupKind::Warning,
                    Some(8.0),
                );
            }
        }
    }
    fn run(&mut self, cx: &mut Cx, command: Command) {
        let feedback = command.feedback();
        let is_refresh = matches!(feedback, CommandFeedback::Refresh(_));
        if self.busy && (self.mutating || is_refresh) {
            return;
        }
        let Some(service) = Service::current() else {
            return;
        };
        self.busy = true;
        self.mutating = !is_refresh;
        self.request = next_request();
        let request = self.request;
        let owner = service.owner.clone();
        let feed = self.feed.clone();
        self.in_flight = match &command {
            Command::Send(p) => Some(sent_kind_for_pending(p)),
            Command::Comment(post, body) => Some(SentKind::Reply {
                key: ReplyDraftKey { room: post.room.clone(), post: post.id.clone() }, body: body.clone(),
            }),
            Command::Retry => service.pending().ok().flatten().map(|p| sent_kind_for_pending(&p)),
            _ => None,
        };
        if feedback == CommandFeedback::Default {
            self.status = match &command {
                Command::Send(_) => crate::i18n::tr("Encrypting and publishing…"),
                Command::FileTransfer(_) => crate::i18n::tr("Opening private File Transfer…"),
                _ => crate::i18n::tr("Updating Moments…"),
            }
            .into();
        }
        spawn_async_task(async move {
            let result=async {
                Ok(match command {
                    Command::Refresh{older,..}=>Outcome::Feed(service.load(feed,older).await?),
                    Command::Prepare=>{let id=if let Some(p)=service.pending()?.filter(|p|p.confirmed.is_none()){p.room}else{service.ensure_timeline().await?};Outcome::Ready(service.validate(&id).await?)},
                    Command::RetrySetup=>{let id=service.retry_timeline_setup().await?;Outcome::Ready(service.validate(&id).await?)},
                    Command::Audience=>{let id=if let Some(p)=service.pending()?.filter(|p|p.confirmed.is_none()){p.room}else{service.ensure_timeline().await?};Outcome::Ready(service.validate(&id).await?)},
                    Command::Send(p)=>{let kind=sent_kind_for_pending(&p);service.send(p).await?;Outcome::Sent(kind)},
                    Command::Retry=>{let p=service.pending()?.ok_or_else(||anyhow::anyhow!(crate::i18n::tr("No saved operation to retry.")))?;let kind=sent_kind_for_pending(&p);service.send(p).await?;if let SentKind::Reply{key,body}=&kind{let _=super::backend::clear_reply_draft_if_matches(&service.owner,key,body);}Outcome::Sent(kind)},
                    Command::Discard=>{service.discard_pending().await?;Outcome::Changed},
                    Command::Review(room,audience)=>{service.review_pending(&room,&audience).await?;Outcome::Changed},
                    Command::Comment(post,body)=>{let key=ReplyDraftKey{room:post.room.clone(),post:post.id.clone()};service.interact(&post,super::model::comment_content(&body,&post.id),"m.room.message",TransactionId::new()).await?;let _=super::backend::clear_reply_draft_if_matches(&service.owner,&key,&body);Outcome::Sent(SentKind::Reply{key,body})},
                    Command::Edit(entry,body)=>{
                        anyhow::ensure!(entry.sender==service.owner,crate::i18n::tr("Only your own text can be edited."));
                        let timeline=service.validate(&entry.room).await?;let mut content=entry.content.clone();content["body"]=serde_json::json!(body);
                        let wire=serde_json::json!({"msgtype":"m.text","body":format!("* {body}"),"m.new_content":content,"m.relates_to":{"rel_type":"m.replace","event_id":entry.id}});
                        let id=entry.id.clone();service.send(Pending{transaction:TransactionId::new(),room:entry.room,audience:timeline.audience,event_type:"m.room.message".into(),content:wire,is_post:false,draft_body:None,paths:vec![],assets:vec![],confirmed:None}).await?;Outcome::Sent(SentKind::Edit(id))
                    },
                    Command::Like(post)=>{service.like(&post,TransactionId::new()).await?;Outcome::Changed},
                    Command::Redact(room,ids)=>{service.redact(&room,ids).await?;Outcome::Changed},
                    Command::Invite(room,user)=>{service.membership(&room,&user,true).await?;Outcome::Ready(service.validate(&room).await?)},
                    Command::Remove(room,user)=>{service.membership(&room,&user,false).await?;Outcome::Ready(service.validate(&room).await?)},
                    Command::Invitation(room,accept)=>{service.invitation(&room,accept).await?;let mut feed=feed;feed.timelines.remove(&room);Outcome::Feed(service.load(feed,false).await?)},
                    Command::Choose(room)=>{service.choose(room.clone()).await?;Outcome::Ready(service.validate(&room).await?)},
                    Command::Hide(author,hidden)=>{service.hide(author,hidden).await?;Outcome::Feed(service.load(feed,false).await?)},
                    Command::ShareWithDmContacts(share)=>{service.set_share_with_dm_contacts(share).await?;Outcome::Feed(service.load(feed,false).await?)},
                    Command::Seen(ids)=>{service.mark_seen(ids).await?;Outcome::Changed},
                    Command::FileTransfer(new)=>Outcome::Transfer(if new {service.new_file_transfer().await?}else{service.file_transfer().await?}),
                })
            }.await.map_err(|e:anyhow::Error|e.to_string());
            Cx::post_action(Completed {
                owner,
                request,
                feedback,
                result,
            });
        });
        self.redraw(cx);
    }
    fn back(&mut self, cx: &mut Cx) {
        // Editing must never consume navigation, including stale edit state
        // left behind after a post disappears from the detail page.
        if self.page == Page::Details || self.editing.is_some() {
            self.leave_detail(cx);
        }
        if self.page == Page::Feed || self.page == Page::Transfer {
            cx.action(MomentsAction::Close);
            return;
        }
        if self.page == Page::Compose { self.save_draft(cx); }
        self.page = if self.page == Page::Audience {
            self.audience_return
        } else {
            Page::Feed
        };
        self.redraw(cx);
    }
    fn open_detail(&mut self, cx: &mut Cx, post: Entry) {
        if self.page == Page::Details { self.leave_detail(cx); }
        self.media_index = 0;
        self.editing = None;
        self.page = Page::Details;
        self.detail = Some(post.clone());
        self.restore_current_reply(cx);
        if !self.feed.preferences.seen.contains(&post.id) {
            self.feed.preferences.seen.insert(post.id.clone());
            self.run(cx, Command::Seen(vec![post.id]));
        }
        self.redraw(cx);
    }
    /// The side of a square cell in a 3-column photo grid, measured from the grid
    /// as just drawn (list items' areas are only valid right after drawing them).
    fn measure_album_side(cx: &mut Cx, album: &ViewRef) -> Option<f64> {
        let width = album.area().rect(cx).size.x;
        (width > 0.0).then(|| ((width - 2.0 * ALBUM_GAP) / 3.0).floor())
    }

    /// Makes the visible rows of a 3-column photo grid `side` tall, so its
    /// photos form squares that follow the grid's width.
    fn square_album_rows(
        cx: &mut Cx,
        album: &ViewRef,
        photo_count: usize,
        side: f64,
        applied: &mut HashMap<WidgetUid, f64>,
    ) {
        if side <= 0.0 {
            return;
        }
        for (id, first) in [(ids!(row0), 0), (ids!(row1), 3), (ids!(row2), 6)] {
            let row = album.view(cx, id);
            if photo_count > first && applied.get(&row.widget_uid()) != Some(&side) {
                // Set the walk directly: this runs mid-draw (inside the feed's PortalList),
                // where re-entering the script VM via `script_apply_eval!` would panic.
                if let Some(mut view) = row.borrow_mut() {
                    view.walk.height = Size::Fixed(side);
                }
                applied.insert(row.widget_uid(), side);
            }
        }
    }

    /// Shows the picked photos/videos as thumbnails in the composer,
    /// reloading them from disk only when the selection changes.
    fn sync_compose_album(&mut self, cx: &mut Cx) {
        let album = self.view(cx, ids!(compose_album));
        Self::square_album_rows(cx, &album, self.paths.len(), self.compose_album_side, &mut self.album_row_heights);
        if self.album_paths == self.paths {
            return;
        }
        self.album_paths = self.paths.clone();
        let count = self.paths.len();
        self.view(cx, ids!(compose_album)).set_visible(cx, count > 0);
        for (id, n) in [(ids!(compose_album.row0), 0), (ids!(compose_album.row1), 3), (ids!(compose_album.row2), 6)] {
            self.view(cx, id).set_visible(cx, count > n);
        }
        let slots = [ids!(c0), ids!(c1), ids!(c2), ids!(c3), ids!(c4), ids!(c5), ids!(c6), ids!(c7), ids!(c8)];
        for (i, id) in slots.iter().enumerate() {
            let photo = self.text_or_image(cx, *id);
            photo.set_visible(cx, i < count);
            let Some(path) = self.paths.get(i) else { continue };
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            // Videos (or anything the image loader can't decode) show their file name instead.
            let shown = photo.show_image(cx, None, |cx, image| {
                image.load_image_file_by_path(cx, path)?;
                Ok::<_, ImageError>(image.size_in_pixels(cx).unwrap_or((1, 1)))
            });
            if shown.is_err() {
                photo.show_text(cx, name);
            }
        }
    }

    fn pick_media(&mut self) {
        if self.paths.len() >= MAX_MEDIA {
            self.status = crate::i18n::tr("Choose at most nine photos or videos.").into();
            return;
        }
        let Some(owner) = self.owner.clone() else {
            return;
        };
        let session = self.session;
        let result = robius_file_picker::FileDialog::new().pick_image_or_video(move |picked| {
            let result = (|| -> Result<Option<PathBuf>, String> {
                let Some(file) = picked.map_err(|e| e.to_string())? else {
                    return Ok(None);
                };
                let local = file.into_local_file().map_err(|e| e.to_string())?;
                let size = std::fs::metadata(local.path())
                    .map_err(|e| e.to_string())?
                    .len();
                if size > 25 * 1024 * 1024 {
                    return Err(crate::i18n::tr("Each photo or video must be at most 25 MB.").into());
                }
                let dir = crate::persistence::persistent_state_dir(&owner)
                    .join("moments-drafts")
                    .join(TransactionId::new().as_str());
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = dir.join(local.path().file_name().unwrap_or_default());
                std::fs::copy(local.path(), &path).map_err(|e| e.to_string())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                        .map_err(|e| e.to_string())?;
                }
                Ok(Some(path))
            })();
            match result {
                Ok(None) => {}
                Ok(Some(path)) => Cx::post_action(Picked {
                    owner,
                    session,
                    result: Ok(path),
                }),
                Err(e) => Cx::post_action(Picked {
                    owner,
                    session,
                    result: Err(e),
                }),
            }
        });
        if let Err(e) = result {
            self.status = e.to_string();
        }
    }
    fn show_media(
        cx: &mut Cx,
        widget: &crate::shared::text_or_image::TextOrImageRef,
        asset: &Asset,
        cache: &mut MediaCache,
        full: bool,
    ) {
        use matrix_sdk::media::MediaFormat;
        if !asset.mimetype.starts_with("image/") {
            widget.show_text(cx, &crate::i18n::format("Video · {0}", &[("0", (asset.name).to_string())]));
            return;
        }
        let source = ruma::events::room::MediaSource::Encrypted(Box::new(asset.file.clone()));
        let format = if full {
            MediaFormat::File
        } else {
            MediaFormat::Thumbnail(matrix_sdk::media::MediaThumbnailSettings::new(
                ruma::uint!(240),
                ruma::uint!(240),
            ))
        };
        match cache.try_get_media_or_fetch(&source, format) {
            (MediaCacheEntry::Loaded(data), _) => {
                let key = format!("{}#moments-{full}", media_source_mxc(&source));
                if widget
                    .show_image(cx, Some(source), |cx, img| {
                        crate::utils::load_image_with_cache_key(
                            &img,
                            cx,
                            std::path::Path::new(&key),
                            data,
                        )
                        .map(|()| img.size_in_pixels(cx).unwrap_or_default())
                    })
                    .is_err()
                {
                    widget.show_text(cx, crate::i18n::tr("Image unavailable"));
                }
            }
            (MediaCacheEntry::Requested, _) => widget.show_text(cx, crate::i18n::tr("Loading photo…")),
            (MediaCacheEntry::Failed(_), _) => {
                widget.show_text(cx, crate::i18n::tr("Photo unavailable · reopen to retry"))
            }
        }
    }
}

impl Widget for MomentsPanel {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        if self.owner.is_none() {
            return;
        }
        if self.owner != current_user_id() {
            self.reset(cx);
            cx.action(MomentsAction::Close);
            return;
        }
        if event.back_pressed()
            || matches!(
                event,
                Event::KeyDown(KeyEvent {
                    key_code: KeyCode::Escape,
                    ..
                })
            )
            || matches!(event,Event::Scroll(e) if self.back_swipe.update(e))
        {
            self.back(cx);
            return;
        }
        if self.timer.is_event(event).is_some()
            && !self.busy
            && matches!(self.page, Page::Feed | Page::Details)
        {
            self.run(
                cx,
                Command::Refresh {
                    older: false,
                    origin: RefreshOrigin::Automatic,
                },
            );
        }
        self.view.handle_event(cx, event, scope);
        if matches!(event, Event::Signal) {
            self.redraw(cx);
        }
        let Event::Actions(actions) = event else {
            return;
        };
        for action in actions {
            if let Some(done) = action.downcast_ref::<Completed>() {
                if Some(&done.owner) != self.owner.as_ref() || done.request != self.request {
                    continue;
                }
                self.busy = false;
                self.in_flight = None;
                self.pending = Service::current()
                    .and_then(|s| s.pending().ok().flatten())
                    .filter(|p| p.confirmed.is_none());
                match &done.result {
                    Err(e) => match done.feedback {
                        CommandFeedback::Refresh(origin) => self.refresh_failed(origin, e),
                        CommandFeedback::Silent => {
                            log!("Silent Moments operation failed: {e}");
                        }
                        CommandFeedback::Default => {
                            self.status = crate::i18n::format(
                                "{e} Refresh or retry to continue.",
                                &[("e", (e).to_string())],
                            )
                        }
                    },
                    Ok(Outcome::Feed(feed)) => {
                        self.feed = feed.clone();
                        if let CommandFeedback::Refresh(origin) = done.feedback {
                            if feed.errors.is_empty() {
                                self.refresh_succeeded(origin);
                            } else {
                                self.refresh_failed(origin, &feed.errors.join("\n"));
                                self.has_loaded_feed = true;
                            }
                        } else {
                            self.status = feed.errors.join("\n");
                            self.has_loaded_feed = true;
                        }
                    }
                    Ok(Outcome::Ready(t)) => {
                        self.timeline = Some(t.clone());
                        self.status.clear();
                        if self.page == Page::Compose && self.pending.is_some() {
                            self.status = crate::i18n::tr("A saved send is waiting. Retry sends its saved content. Discard the retry to write a new post.").into();
                        }
                    }
                    Ok(Outcome::Changed) => {
                        if done.feedback != CommandFeedback::Silent {
                            self.status = crate::i18n::tr("Updated.").into();
                        }
                        if self.page == Page::Details || self.page == Page::Feed {
                            self.run(
                                cx,
                                Command::Refresh {
                                    older: false,
                                    origin: RefreshOrigin::FollowUp,
                                },
                            );
                        }
                    }
                    Ok(Outcome::Sent(kind)) => {
                        self.status = crate::i18n::tr("Sent.").into();
                        match kind {
                            SentKind::Post { body, paths } => {
                                if post_draft_matches(body, paths,
                                    &self.text_input(cx, ids!(moments_body)).text(), &self.paths) {
                                    self.text_input(cx, ids!(moments_body)).set_text(cx, "");
                                    self.paths.clear();
                                    if self.page == Page::Compose { self.page = Page::Feed; }
                                }
                            }
                            SentKind::Reply { key, body } => {
                                if self.detail_key().as_ref() == Some(key) && self.editing.is_none()
                                    && self.text_input(cx, ids!(moments_comment)).text() == *body {
                                    self.restore_current_reply(cx);
                                }
                            }
                            SentKind::Edit(id) => {
                                if self.editing.as_ref().is_some_and(|entry| &entry.id == id) {
                                    self.editing = None;
                                    if self.page == Page::Details { self.restore_current_reply(cx); }
                                }
                            }
                            SentKind::Other => {}
                        }
                        self.run(
                            cx,
                            Command::Refresh {
                                older: false,
                                origin: RefreshOrigin::FollowUp,
                            },
                        );
                    }
                    Ok(Outcome::Transfer(id)) => {
                        cx.widget_action(
                            self.widget_uid(),
                            crate::home::rooms_list::RoomsListAction::Selected(
                                crate::app::SelectedRoom::JoinedRoom {
                                    room_name_id: crate::utils::RoomNameId::from((
                                        Some(matrix_sdk::RoomDisplayName::Named(
                                            crate::i18n::tr("File Transfer").into(),
                                        )),
                                        id.clone(),
                                    )),
                                },
                            ),
                        );
                        cx.action(MomentsAction::Close);
                    }
                }
                self.redraw(cx);
            }
            if let Some(picked) = action.downcast_ref::<Picked>() {
                if Some(&picked.owner) == self.owner.as_ref() && picked.session == self.session {
                    match &picked.result {
                        Ok(path) => {
                            self.paths.push(path.clone());
                            self.save_draft(cx);
                        }
                        Err(e) => self.status = e.clone(),
                    }
                    self.redraw(cx);
                }
            }
        }
        if self.button(cx, ids!(header.back)).clicked(actions) {
            self.back(cx);
            return;
        }
        if self.page == Page::Details
            && self.text_input(cx, ids!(moments_comment)).changed(actions).is_some() {
            self.save_current_reply(cx);
        }
        if self.page == Page::Compose
            && self.text_input(cx, ids!(moments_body)).changed(actions).is_some() {
            self.save_draft(cx);
        }
        if self.busy && self.mutating {
            return;
        }
        match self.page {
            Page::Feed => {
                if self.button(cx, ids!(moments_refresh)).clicked(actions) {
                    self.run(
                        cx,
                        Command::Refresh {
                            older: false,
                            origin: RefreshOrigin::Manual,
                        },
                    );
                }
                if self.button(cx, ids!(moments_more)).clicked(actions) {
                    self.run(
                        cx,
                        Command::Refresh {
                            older: true,
                            origin: RefreshOrigin::Manual,
                        },
                    );
                }
                if self.button(cx, ids!(moments_compose)).clicked(actions) {
                    self.page = Page::Compose;
                    self.run(cx, Command::Prepare);
                }
                if self.button(cx, ids!(moments_audience)).clicked(actions) {
                    self.audience_return = Page::Feed;
                    self.page = Page::Audience;
                    self.run(cx, Command::Audience);
                }
                if self.button(cx, ids!(moments_invites)).clicked(actions) {
                    self.page = Page::Invitations;
                    self.run(
                        cx,
                        Command::Refresh {
                            older: false,
                            origin: RefreshOrigin::Manual,
                        },
                    );
                }
                let list = self.portal_list(cx, ids!(moments_feed));
                let mut used = BTreeSet::new();
                for (index, row) in list.items_with_actions(actions) {
                    if !used.insert(index) || list.was_scrolling() {
                        continue;
                    }
                    let photo_clicked = [
                        ids!(a0),
                        ids!(a1),
                        ids!(a2),
                        ids!(a3),
                        ids!(a4),
                        ids!(a5),
                        ids!(a6),
                        ids!(a7),
                        ids!(a8),
                    ]
                    .into_iter()
                    .position(|id| {
                        let photo = row.text_or_image(cx, id);
                        actions.iter().any(|a| {
                            matches!(
                                a.as_widget_action()
                                    .widget_uid_eq(photo.widget_uid())
                                    .cast(),
                                TextOrImageAction::Clicked(_)
                            )
                        })
                    });
                    if photo_clicked.is_some()
                        || actions.iter().any(|a| {
                            matches!(
                                a.as_widget_action().widget_uid_eq(row.widget_uid()).cast(),
                                NavigationBarButtonAction::Clicked
                            )
                        })
                    {
                        if let Some(post) = index
                            .checked_sub(1)
                            .and_then(|i| self.posts.get(i))
                            .cloned()
                        {
                            self.open_detail(cx, post);
                            self.media_index = photo_clicked.unwrap_or(0);
                            break;
                        }
                    }
                }
            }
            Page::Compose => {
                if self.button(cx, ids!(moments_add_media)).clicked(actions) {
                    self.pick_media();
                }
                if self.button(cx, ids!(moments_clear_media)).clicked(actions) {
                    self.paths.clear();
                    self.save_draft(cx);
                }
                if self.button(cx, ids!(compose_audience)).clicked(actions) {
                    self.audience_return = Page::Compose;
                    self.page = Page::Audience;
                    self.run(cx, Command::Audience);
                }
                if self.button(cx, ids!(moments_discard)).clicked(actions) {
                    self.run(cx, Command::Discard);
                }
                if self.button(cx, ids!(moments_retry)).clicked(actions) {
                    self.run(cx, Command::Retry);
                }
                if self.button(cx, ids!(moments_publish)).clicked(actions) {
                    let body = self.text_input(cx, ids!(moments_body)).text();
                    if body.trim().is_empty() && self.paths.is_empty() {
                        self.status = crate::i18n::tr("Write a post or add a photo.").into();
                    } else if let Some(t) = &self.timeline {
                        self.save_draft(cx);
                        self.run(
                            cx,
                            Command::Send(Pending {
                                transaction: TransactionId::new(),
                                room: t.room.clone(),
                                audience: t.audience.clone(),
                                event_type: "m.room.message".into(),
                                content: super::model::post_content(&body, &[]),
                                is_post: true,
                                draft_body: Some(body.clone()),
                                paths: self.paths.clone(),
                                assets: vec![],
                                confirmed: None,
                            }),
                        );
                    }
                }
            }
            Page::Details => {
                if let Some(post) = self.detail.clone() {
                    if self.button(cx, ids!(moments_like)).clicked(actions) {
                        let own = self.feed.timelines.get(&post.room).and_then(|t| {
                            t.index
                                .likes(&post)
                                .get(self.owner.as_ref().unwrap())
                                .cloned()
                        });
                        self.run(
                            cx,
                            match own {
                                Some(ids) => Command::Redact(post.room.clone(), ids),
                                None => Command::Like(post.clone()),
                            },
                        );
                    }
                    if self.button(cx, ids!(moments_hide)).clicked(actions) {
                        self.leave_detail(cx);
                        self.page = Page::Feed;
                        self.run(cx, Command::Hide(post.sender.clone(), true));
                    }
                    if self.button(cx, ids!(moments_edit)).clicked(actions) {
                        self.start_edit(cx, post.clone());
                    }
                    if self.button(cx, ids!(moments_delete)).clicked(actions) {
                        self.run(
                            cx,
                            Command::Redact(post.room.clone(), vec![post.id.clone()]),
                        );
                        self.leave_detail(cx);
                        self.page = Page::Feed;
                    }
                    if self.button(cx, ids!(moments_comment_send)).clicked(actions)
                        || self
                            .text_input(cx, ids!(moments_comment))
                            .returned(actions)
                            .is_some()
                    {
                        let body = self.text_input(cx, ids!(moments_comment)).text();
                        if !body.trim().is_empty() {
                            self.save_current_reply(cx);
                            self.run(
                                cx,
                                match self.editing.clone() {
                                    Some(e) => Command::Edit(e, body),
                                    None => Command::Comment(post.clone(), body),
                                },
                            );
                        }
                    }
                    let media = post.media();
                    if !media.is_empty() {
                        if self.button(cx, ids!(media_next)).clicked(actions) {
                            self.media_index = (self.media_index + 1) % media.len();
                        }
                        if self.button(cx, ids!(media_previous)).clicked(actions) {
                            self.media_index = (self.media_index + media.len() - 1) % media.len();
                        }
                        if self.button(cx, ids!(media_download)).clicked(actions) {
                            let a = &media[self.media_index];
                            start_attachment_download(
                                DownloadableAttachment {
                                    media_source: ruma::events::room::MediaSource::Encrypted(
                                        Box::new(a.file.clone()),
                                    ),
                                    filename: a.name.clone(),
                                    size: Some(a.size),
                                    kind: if a.mimetype.starts_with("video/") {
                                        DownloadKind::Video
                                    } else {
                                        DownloadKind::Image
                                    },
                                },
                                None,
                            );
                        }
                    }
                    let mut used = BTreeSet::new();
                    for (index, row) in self
                        .portal_list(cx, ids!(comments))
                        .items_with_actions(actions)
                    {
                        if !used.insert(index) {
                            continue;
                        }
                        if let Some(comment) = self.comments.get(index).cloned() {
                            if row.button(cx, ids!(comment_edit)).clicked(actions) {
                                self.start_edit(cx, comment.clone());
                            }
                            if row.button(cx, ids!(comment_delete)).clicked(actions) {
                                if self.editing.as_ref().is_some_and(|entry| entry.id == comment.id) {
                                    self.editing = None;
                                    self.restore_current_reply(cx);
                                }
                                self.run(cx, Command::Redact(comment.room, vec![comment.id]));
                            }
                        }
                    }
                }
            }
            Page::Audience => {
                if let Some(share) = self.check_box(cx, ids!(share_dm_contacts)).changed(actions) {
                    self.run(cx, Command::ShareWithDmContacts(share));
                }
                if self.button(cx, ids!(moments_retry_setup)).clicked(actions) {
                    self.run(cx, Command::RetrySetup);
                }
                if self.button(cx, ids!(audience_review)).clicked(actions) {
                    if let Some(t) = &self.timeline {
                        self.run(cx, Command::Review(t.room.clone(), t.audience.clone()));
                    }
                }
                if self.button(cx, ids!(audience_invite)).clicked(actions) {
                    match OwnedUserId::try_from(
                        self.text_input(cx, ids!(audience_user)).text().trim(),
                    ) {
                        Ok(user) => {
                            if let Some(t) = &self.timeline {
                                self.run(cx, Command::Invite(t.room.clone(), user));
                            }
                        }
                        Err(_) => {
                            self.status = crate::i18n::tr("Enter a Matrix ID such as @friend:matrix.org.").into()
                        }
                    }
                }
                let members = self
                    .timeline
                    .as_ref()
                    .map(|t| t.members.clone())
                    .unwrap_or_default();
                let choices: Vec<_> = self
                    .feed
                    .own(self.owner.as_ref().unwrap())
                    .iter()
                    .map(|t| t.room.clone())
                    .collect();
                let hidden: Vec<_> = self.feed.preferences.hidden.iter().cloned().collect();
                let mut used = BTreeSet::new();
                for (index, row) in self
                    .portal_list(cx, ids!(audience_list))
                    .items_with_actions(actions)
                {
                    if !used.insert(index) {
                        continue;
                    }
                    if row.button(cx, ids!(remove_viewer)).clicked(actions) {
                        if let (Some(t), Some(m)) = (&self.timeline, members.get(index)) {
                            self.run(cx, Command::Remove(t.room.clone(), m.id.clone()));
                        }
                    }
                    if row.button(cx, ids!(choose_timeline)).clicked(actions) {
                        if let Some(id) = index
                            .checked_sub(members.len())
                            .and_then(|i| choices.get(i))
                        {
                            self.run(cx, Command::Choose(id.clone()));
                        }
                    }
                    if row.button(cx, ids!(unhide_author)).clicked(actions) {
                        if let Some(user) = index
                            .checked_sub(members.len() + choices.len())
                            .and_then(|i| hidden.get(i))
                        {
                            self.run(cx, Command::Hide(user.clone(), false));
                        }
                    }
                }
            }
            Page::Invitations => {
                let invites: Vec<_> = self
                    .feed
                    .timelines
                    .values()
                    .filter(|t| t.invited)
                    .map(|t| t.room.clone())
                    .collect();
                let mut used = BTreeSet::new();
                for (index, row) in self
                    .portal_list(cx, ids!(invitation_list))
                    .items_with_actions(actions)
                {
                    if !used.insert(index) {
                        continue;
                    }
                    if let Some(id) = invites.get(index) {
                        if row.button(cx, ids!(accept_moments)).clicked(actions) {
                            self.run(cx, Command::Invitation(id.clone(), true));
                        }
                        if row.button(cx, ids!(reject_moments)).clicked(actions) {
                            self.run(cx, Command::Invitation(id.clone(), false));
                        }
                    }
                }
            }
            Page::Transfer => {
                if self.button(cx, ids!(transfer_retry)).clicked(actions) {
                    self.run(cx, Command::FileTransfer(false));
                }
                if self.button(cx, ids!(transfer_new)).clicked(actions) {
                    self.run(cx, Command::FileTransfer(true));
                }
            }
        }
        self.redraw(cx);
    }
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        if self.owner.is_some() && self.owner != current_user_id() {
            self.reset(cx);
        }
        for (id, page) in [
            (ids!(feed_page), Page::Feed),
            (ids!(compose_page), Page::Compose),
            (ids!(details_page), Page::Details),
            (ids!(audience_page), Page::Audience),
            (ids!(invitations_page), Page::Invitations),
            (ids!(transfer_page), Page::Transfer),
        ] {
            self.view(cx, id).set_visible(cx, self.page == page);
        }
        self.label(cx, ids!(header.title)).set_text(
            cx,
            match self.page {
                Page::Feed => {
                    if self.author == self.owner && self.owner.is_some() {
                        crate::i18n::tr("My Posts")
                    } else {
                        crate::i18n::tr("Moments")
                    }
                }
                Page::Compose => crate::i18n::tr("New Moment"),
                Page::Details => crate::i18n::tr("Moment"),
                Page::Audience => crate::i18n::tr("Timeline Audience"),
                Page::Invitations => crate::i18n::tr("Timeline Invitations"),
                Page::Transfer => crate::i18n::tr("File Transfer"),
            },
        );
        self.posts = self.feed.posts(self.author.as_deref());
        let unavailable: usize = self
            .feed
            .timelines
            .values()
            .map(|t| t.index.unavailable())
            .sum();
        let unvisited = self.feed.undiscovered
            + self
                .feed
                .timelines
                .values()
                .filter(|t| !t.invited && (!t.loaded || t.refresh_cursor.is_some()))
                .count();
        let status = if !self.status.is_empty() {
            self.status.clone()
        } else if !self.refresh_notice.is_empty() {
            self.refresh_notice.clone()
        } else if unavailable > 0 {
            crate::i18n::format("{unavailable} encrypted events unavailable. Refresh after recovering keys.", &[("unavailable", (unavailable).to_string())])
        } else if unvisited > 0 {
            crate::i18n::format("{unvisited} timelines not loaded yet. Load more to continue.", &[("unvisited", (unvisited).to_string())])
        } else {
            String::new()
        };
        self.label(cx, ids!(moments_status)).set_text(cx, &status);
        self.label(cx, ids!(moments_status))
            .set_visible(cx, !status.is_empty());
        self.button(cx, ids!(moments_refresh)).set_text(
            cx,
            if self.refresh_notice.is_empty() {
                crate::i18n::tr("Refresh")
            } else {
                crate::i18n::tr("Retry")
            },
        );
        let invites: Vec<_> = self
            .feed
            .timelines
            .values()
            .filter(|t| t.invited)
            .cloned()
            .collect();
        self.button(cx, ids!(moments_invites))
            .set_text(cx, &crate::i18n::format("Invites ({0})", &[("0", (invites.len()).to_string())]));
        self.label(cx, ids!(invitation_empty))
            .set_visible(cx, invites.is_empty());
        let members = self
            .timeline
            .as_ref()
            .map(|t| t.members.clone())
            .unwrap_or_default();
        let manages_audience = self
            .timeline
            .as_ref()
            .is_some_and(|t| Some(&t.author) == self.owner.as_ref());
        self.view(cx, ids!(audience_invite_controls))
            .set_visible(cx, manages_audience);
        self.view(cx, ids!(setup_recovery))
            .set_visible(cx, self.timeline.is_none() && !self.busy);
        let choices: Vec<_> = self
            .owner
            .as_ref()
            .map(|o| self.feed.own(o).into_iter().cloned().collect())
            .unwrap_or_default();
        let hidden: Vec<_> = self.feed.preferences.hidden.iter().cloned().collect();
        // Mirror the saved preference, unless a change to it is still being applied.
        let share_toggle = self.check_box(cx, ids!(share_dm_contacts));
        if !self.busy && share_toggle.active(cx) != self.feed.preferences.share_with_dm_contacts {
            share_toggle.set_active(cx, self.feed.preferences.share_with_dm_contacts, Animate::No);
        }
        self.label(cx, ids!(audience_name)).set_text(
            cx,
            &self
                .timeline
                .as_ref()
                .map(|t| {
                    crate::i18n::format("{0} · {1} viewers / invitations", &[("0", (t.name).to_string()), ("1", (t.members.len().saturating_sub(1)).to_string())])
                })
                .unwrap_or_else(|| crate::i18n::tr("Choose or create your timeline").into()),
        );
        self.label(cx, ids!(composer_audience)).set_text(
            cx,
            &crate::i18n::format("Timeline audience: {0}", &[("0", (if members.len() <= 1 {
                    crate::i18n::tr("Only me").into()
                } else {
                    members
                        .iter()
                        .filter(|m| Some(&m.id) != self.owner.as_ref())
                        .map(|m| format!("{}{}", m.name, if m.invited { crate::i18n::tr(" (invited)") } else { "" }))
                        .collect::<Vec<_>>()
                        .join(", ")
                }).to_string())]),
        );
        self.sync_compose_album(cx);
        self.label(cx, ids!(selected_media)).set_text(
            cx,
            &crate::i18n::format("{0} / 9 selected\n{1}", &[("0", (self.paths.len()).to_string()), ("1", (self.paths
                    .iter()
                    .filter_map(|p| p.file_name())
                    .map(|n| n.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(", ")).to_string())]),
        );
        let pending = self.pending.is_some();
        self.button(cx, ids!(moments_publish))
            .set_visible(cx, !pending);
        self.button(cx, ids!(moments_publish))
            .set_enabled(cx, !self.busy && self.timeline.is_some());
        if let Some(p) = &self.pending {
            self.button(cx, ids!(moments_retry)).set_text(
                cx,
                &crate::i18n::format("Retry saved {0}", &[("0", (if p.is_post { crate::i18n::tr("post") } else { crate::i18n::tr("interaction") }).to_string())]),
            );
        }
        self.button(cx, ids!(moments_retry))
            .set_visible(cx, pending);
        self.button(cx, ids!(moments_discard))
            .set_visible(cx, pending);
        self.label(cx, ids!(retry_hint)).set_visible(cx, pending);
        self.button(cx, ids!(audience_review))
            .set_visible(cx, pending);
        self.label(cx, ids!(editor_hint))
            .set_visible(cx, self.editing.is_some());
        self.label(cx, ids!(editor_hint))
            .set_text(cx, crate::i18n::tr("Editing your text · Back discards changes and returns"));
        if let Some(post) = self.detail.clone() {
            if let Some(t) = self.feed.timelines.get(&post.room) {
                if let Some(updated) = t
                    .index
                    .posts(&post.room, &t.author)
                    .into_iter()
                    .find(|p| p.id == post.id)
                {
                    self.detail = Some(updated);
                } else if t.loaded {
                    self.leave_detail(cx);
                    self.status = crate::i18n::tr("This post was removed.").into();
                    self.page = Page::Feed;
                }
            }
        }
        if let Some(post) = &self.detail {
            self.label(cx, ids!(detail_author)).set_text(
                cx,
                &self
                    .feed
                    .timelines
                    .get(&post.room)
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| post.sender.to_string()),
            );
            self.label(cx, ids!(detail_body)).set_text(cx, post.body());
            self.view(cx, ids!(owner_actions))
                .set_visible(cx, Some(&post.sender) == self.owner.as_ref());
            self.button(cx, ids!(moments_hide))
                .set_visible(cx, Some(&post.sender) != self.owner.as_ref());
            let likes = self
                .feed
                .timelines
                .get(&post.room)
                .map(|t| t.index.likes(post))
                .unwrap_or_default();
            self.button(cx, ids!(moments_like)).set_text(
                cx,
                if self.owner.as_ref().is_some_and(|u| likes.contains_key(u)) {
                    crate::i18n::tr("Unlike")
                } else {
                    crate::i18n::tr("Like")
                },
            );
            self.label(cx, ids!(detail_likes)).set_text(
                cx,
                &crate::i18n::format("{0} likes{1}", &[("0", (likes.len()).to_string()), ("1", (if likes.is_empty() {
                        String::new()
                    } else {
                        format!(
                            ": {}",
                            likes
                                .keys()
                                .map(|u| u.localpart())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }).to_string())]),
            );
            self.comments = self
                .feed
                .timelines
                .get(&post.room)
                .map(|t| t.index.comments(post))
                .unwrap_or_default();
            let assets = post.media();
            self.media_index = self.media_index.min(assets.len().saturating_sub(1));
            self.view(cx, ids!(media_controls))
                .set_visible(cx, !assets.is_empty());
            let preview = self.text_or_image(cx, ids!(detail_media));
            preview.set_visible(cx, !assets.is_empty());
            self.label(cx, ids!(detail_meta)).set_text(
                cx,
                &format!(
                    "{}{}{}",
                    date(post.timestamp),
                    if post.edited { crate::i18n::tr(" · Edited") } else { "" },
                    if assets.is_empty() {
                        String::new()
                    } else {
                        crate::i18n::format(" · Media {0} / {1}", &[("0", (self.media_index + 1).to_string()), ("1", (assets.len()).to_string())])
                    }
                ),
            );
            if let (Some(asset), Some(cache)) = (assets.get(self.media_index), self.media.as_mut())
            {
                Self::show_media(cx, &preview, asset, cache, true);
            }
        }
        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            let Some(mut list) = step.borrow_mut::<PortalList>() else {
                continue;
            };
            match self.page {
                Page::Feed => {
                    list.set_item_range(cx, 0, self.posts.len().max(1) + 1);
                    while let Some(index) = list.next_visible_item(cx) {
                        if index == 0 {
                            let row = list.item(cx, index, id!(Cover));
                            let name = self
                                .author
                                .as_ref()
                                .map(|u| u.localpart().to_owned())
                                .unwrap_or_else(|| crate::i18n::tr("Moments").into());
                            row.label(cx, ids!(cover_name)).set_text(cx, &name);
                            row.draw_all(cx, scope);
                            continue;
                        }
                        let Some(post) = self.posts.get(index - 1) else {
                            let template = if self.posts.is_empty() && index == 1 {
                                id!(Empty)
                            } else {
                                id!(Filler)
                            };
                            list.item(cx, index, template).draw_all(cx, scope);
                            continue;
                        };
                        let row = list.item(cx, index, id!(Post));
                        let timeline = self.feed.timelines.get(&post.room).unwrap();
                        row.avatar(cx, ids!(post_avatar))
                            .show_text(cx, None, None, &timeline.name);
                        row.label(cx, ids!(post_author))
                            .set_text(cx, &timeline.name);
                        row.label(cx, ids!(post_body)).set_text(cx, post.body());
                        row.label(cx, ids!(post_meta)).set_text(
                            cx,
                            &format!(
                                "{}{}{}",
                                date(post.timestamp),
                                if post.edited { crate::i18n::tr(" · Edited") } else { "" },
                                if self.feed.preferences.seen.contains(&post.id) {
                                    ""
                                } else {
                                    crate::i18n::tr(" · New")
                                }
                            ),
                        );
                        row.label(cx, ids!(post_interactions)).set_text(
                            cx,
                            &crate::i18n::format("{0} likes · {1} comments", &[("0", (timeline.index.likes(post).len()).to_string()), ("1", (timeline.index.comments(post).len()).to_string())]),
                        );
                        let media = post.media();
                        row.view(cx, ids!(album)).set_visible(cx, !media.is_empty());
                        let album = row.view(cx, ids!(album));
                        Self::square_album_rows(cx, &album, media.len(), self.feed_album_side, &mut self.album_row_heights);
                        for (id, n) in [(ids!(row0), 0), (ids!(row1), 3), (ids!(row2), 6)] {
                            row.view(cx, id).set_visible(cx, media.len() > n);
                        }
                        for (i, id) in [
                            ids!(a0),
                            ids!(a1),
                            ids!(a2),
                            ids!(a3),
                            ids!(a4),
                            ids!(a5),
                            ids!(a6),
                            ids!(a7),
                            ids!(a8),
                        ]
                        .iter()
                        .enumerate()
                        {
                            let photo = row.text_or_image(cx, *id);
                            photo.set_visible(cx, i < media.len());
                            if let (Some(asset), Some(cache)) = (media.get(i), self.media.as_mut())
                            {
                                Self::show_media(cx, &photo, asset, cache, false);
                            }
                        }
                        row.draw_all(cx, scope);
                        if !media.is_empty()
                            && let Some(side) = Self::measure_album_side(cx, &album)
                            && (side - self.feed_album_side).abs() > 0.5
                        {
                            self.feed_album_side = side;
                            // Mark our area dirty instead of `self.redraw()`, which would
                            // re-borrow the PortalList that is drawing this item.
                            self.view.area().redraw(cx);
                        }
                    }
                }
                Page::Details => {
                    list.set_item_range(cx, 0, self.comments.len());
                    while let Some(i) = list.next_visible_item(cx) {
                        if let Some(comment) = self.comments.get(i) {
                            let row = list.item(cx, i, id!(Comment));
                            row.label(cx, ids!(comment_name)).set_text(
                                cx,
                                &format!(
                                    "{} · {}",
                                    comment.sender.localpart(),
                                    date(comment.timestamp)
                                ),
                            );
                            row.label(cx, ids!(comment_body))
                                .set_text(cx, comment.body());
                            row.view(cx, ids!(comment_actions))
                                .set_visible(cx, Some(&comment.sender) == self.owner.as_ref());
                            row.draw_all(cx, scope);
                        }
                    }
                }
                Page::Audience => {
                    list.set_item_range(cx, 0, members.len() + choices.len() + hidden.len());
                    while let Some(i) = list.next_visible_item(cx) {
                        if let Some(member) = members.get(i) {
                            let row = list.item(cx, i, id!(Member));
                            row.label(cx, ids!(member_name)).set_text(
                                cx,
                                &format!(
                                    "{}{}",
                                    member.name,
                                    if member.invited { crate::i18n::tr(" · Invited") } else { "" }
                                ),
                            );
                            row.label(cx, ids!(member_id))
                                .set_text(cx, member.id.as_str());
                            row.button(cx, ids!(remove_viewer)).set_visible(
                                cx,
                                manages_audience && Some(&member.id) != self.owner.as_ref(),
                            );
                            row.draw_all(cx, scope);
                        } else if let Some(t) =
                            i.checked_sub(members.len()).and_then(|n| choices.get(n))
                        {
                            let row = list.item(cx, i, id!(Choice));
                            row.label(cx, ids!(choice_name))
                                .set_text(cx, &crate::i18n::format("Timeline: {0}", &[("0", (t.room).to_string())]));
                            row.button(cx, ids!(choose_timeline))
                                .set_visible(cx, choices.len() > 1);
                            row.draw_all(cx, scope);
                        } else if let Some(user) = i
                            .checked_sub(members.len() + choices.len())
                            .and_then(|n| hidden.get(n))
                        {
                            let row = list.item(cx, i, id!(Hidden));
                            row.label(cx, ids!(hidden_name))
                                .set_text(cx, &crate::i18n::format("Hidden: {user}", &[("user", (user).to_string())]));
                            row.draw_all(cx, scope);
                        }
                    }
                }
                Page::Invitations => {
                    list.set_item_range(cx, 0, invites.len());
                    while let Some(i) = list.next_visible_item(cx) {
                        if let Some(t) = invites.get(i) {
                            let row = list.item(cx, i, id!(Invitation));
                            row.label(cx, ids!(invitation_author)).set_text(
                                cx,
                                &crate::i18n::format("{0} invited you to their Moments", &[("0", (t.author).to_string())]),
                            );
                            row.draw_all(cx, scope);
                        }
                    }
                }
                _ => {}
            }
        }
        if self.page == Page::Compose && !self.paths.is_empty() {
            let album = self.view(cx, ids!(compose_album));
            if let Some(side) = Self::measure_album_side(cx, &album)
                && (side - self.compose_album_side).abs() > 0.5
            {
                self.compose_album_side = side;
                self.view.area().redraw(cx);
            }
        }
        DrawStep::done()
    }
}
fn date(timestamp: u64) -> String {
    chrono::DateTime::from_timestamp_millis(timestamp.min(i64::MAX as u64) as i64)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%m/%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::post_draft_matches;
    use std::path::PathBuf;

    #[test]
    fn sent_post_only_matches_its_original_draft() {
        let media = vec![PathBuf::from("photo.png")];
        assert!(post_draft_matches("", &media, "", &media));
        assert!(!post_draft_matches("caption", &media, "", &media));
        assert!(!post_draft_matches("caption", &media, "caption", &[]));
    }
}
impl MomentsPanelRef {
    /// Applies `action` to this panel.
    ///
    /// `modal` is the modal hosting this panel, which is opened or closed to match;
    /// it's `None` when the panel lives in its own window instead.
    pub fn action(&self, cx: &mut Cx, modal: Option<&ModalRef>, action: &MomentsAction) {
        let Some(mut inner) = self.borrow_mut() else {
            return;
        };
        if matches!(action, MomentsAction::Close) {
            inner.reset(cx);
            if let Some(modal) = modal { modal.close(cx); }
            return;
        }
        inner.reset(cx);
        inner.owner = current_user_id();
        inner.restore_draft(cx);
        inner.media = Some(MediaCache::new(None));
        inner.timer = cx.start_interval(12.0);
        inner.pending = Service::current()
            .and_then(|s| s.pending().ok().flatten())
            .filter(|p| p.confirmed.is_none());
        if let Some(modal) = modal { modal.open(cx); }
        match action {
            MomentsAction::Open { author } => {
                inner.author = author.clone();
                inner.run(
                    cx,
                    Command::Refresh {
                        older: false,
                        origin: RefreshOrigin::Initial,
                    },
                );
            }
            MomentsAction::Compose { text } => {
                inner.page = Page::Compose;
                inner.text_input(cx, ids!(moments_body)).set_text(cx, text);
                inner.run(cx, Command::Prepare);
            }
            MomentsAction::FileTransfer => {
                inner.page = Page::Transfer;
                inner.run(cx, Command::FileTransfer(false));
            }
            MomentsAction::Close => {}
        }
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;

    #[test]
    fn refresh_failures_are_quiet_until_background_threshold() {
        assert_eq!(
            refresh_failure_notice(RefreshOrigin::Automatic, true, 1, false),
            RefreshFailureNotice::None
        );
        assert_eq!(
            refresh_failure_notice(RefreshOrigin::Automatic, true, 2, false),
            RefreshFailureNotice::None
        );
        assert_eq!(
            refresh_failure_notice(RefreshOrigin::Automatic, true, 3, false),
            RefreshFailureNotice::StaleWarning
        );
        assert_eq!(
            refresh_failure_notice(RefreshOrigin::Automatic, true, 4, true),
            RefreshFailureNotice::None
        );
    }

    #[test]
    fn initial_and_manual_failures_remain_actionable() {
        assert_eq!(
            refresh_failure_notice(RefreshOrigin::Initial, false, 1, false),
            RefreshFailureNotice::InlineUnavailable
        );
        assert_eq!(
            refresh_failure_notice(RefreshOrigin::Manual, true, 1, false),
            RefreshFailureNotice::ImmediateWarning
        );
        assert_eq!(
            refresh_failure_notice(RefreshOrigin::FollowUp, true, 3, false),
            RefreshFailureNotice::StaleWarning
        );
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;

    fn panel() -> (Cx, MomentsPanel) {
        let mut cx = Cx::new(Box::new(|_, _| {}));
        let panel = cx.with_vm(|vm| {
            makepad_widgets::script_mod(vm);
            MomentsPanel::script_new(vm)
        });
        (cx, panel)
    }

    fn edited_post() -> Entry {
        Entry {
            room: ruma::room_id!("!moments:example.org").to_owned(),
            id: ruma::event_id!("$post").to_owned(),
            sender: ruma::user_id!("@author:example.org").to_owned(),
            timestamp: 0,
            content: super::super::model::post_content("Published text", &[]),
            edited: false,
        }
    }

    #[test]
    fn back_during_edit_returns_to_feed_immediately() {
        let (mut cx, mut panel) = panel();
        panel.page = Page::Details;
        panel.detail = Some(edited_post());
        panel.editing = panel.detail.clone();
        panel.back(&mut cx);
        assert!(panel.page == Page::Feed);
        assert!(panel.editing.is_none());
        assert!(panel.detail.is_none());
    }

    #[test]
    fn feed_back_closes_even_with_stale_edit_state() {
        let (mut cx, mut panel) = panel();
        panel.editing = Some(edited_post());
        let actions = cx.capture_actions(|cx| panel.back(cx));
        assert!(actions.iter().any(|a| matches!(
            a.downcast_ref::<MomentsAction>(), Some(MomentsAction::Close)
        )));
        assert!(panel.editing.is_none());
    }

    #[test]
    fn back_does_not_wait_for_in_flight_operation() {
        let (mut cx, mut panel) = panel();
        panel.page = Page::Compose;
        panel.busy = true;
        panel.mutating = true;
        panel.back(&mut cx);
        assert!(panel.page == Page::Feed);
    }

    #[test]
    fn audience_back_returns_to_its_opening_page() {
        let (mut cx, mut panel) = panel();
        for destination in [Page::Feed, Page::Compose] {
            panel.page = Page::Audience;
            panel.audience_return = destination;
            panel.back(&mut cx);
            assert!(panel.page == destination);
        }
    }
}
