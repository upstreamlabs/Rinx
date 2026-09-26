use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use anyhow::{Result, bail, ensure};
use matrix_sdk::{Client, Room, RoomState, room::MessagesOptions};
use ruma::{
    OwnedEventId, OwnedRoomId, OwnedUserId, OwnedTransactionId, TransactionId,
    api::client::{room::create_room, state::get_state_events},
    serde::Raw,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use super::{
    ROOM_TYPE, ACCOUNT_DATA,
    model::{Asset, Entry, Index, MAX_MEDIA, MAX_TEXT, LIKE},
};

// Mutations are ordered within a process. Cross-device creation is reconciled
// against joined rooms and duplicates require an explicit timeline choice.
static WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static REPLY_DRAFT_WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preferences {
    pub timeline: Option<OwnedRoomId>,
    pub file_transfer: Option<OwnedRoomId>,
    #[serde(default)]
    pub hidden: BTreeSet<OwnedUserId>,
    #[serde(default)]
    pub seen: BTreeSet<OwnedEventId>,
    /// Share this account's Moments with its DM contacts; see `super::dm_sharing`.
    /// On unless the user turned it off, like WeChat's friend circle.
    #[serde(default = "default_true")]
    pub share_with_dm_contacts: bool,
}
fn default_true() -> bool {
    true
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            timeline: None,
            file_transfer: None,
            hidden: BTreeSet::new(),
            seen: BTreeSet::new(),
            share_with_dm_contacts: true,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ComposerDraft {
    pub body: String,
    pub paths: Vec<PathBuf>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyDraftKey {
    pub room: OwnedRoomId,
    pub post: OwnedEventId,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ReplyDraft {
    key: ReplyDraftKey,
    body: String,
}
#[derive(Debug, Serialize, Deserialize)]
struct ReplyDrafts {
    version: u8,
    drafts: Vec<ReplyDraft>,
}
impl Default for ReplyDrafts {
    fn default() -> Self { Self { version: 1, drafts: Vec::new() } }
}
impl ReplyDrafts {
    fn get(&self, key: &ReplyDraftKey) -> String {
        self.drafts.iter().find(|draft| &draft.key == key)
            .map_or_else(String::new, |draft| draft.body.clone())
    }
    fn set(&mut self, key: &ReplyDraftKey, body: &str) -> bool {
        if self.get(key) == body { return false; }
        self.drafts.retain(|draft| &draft.key != key);
        if !body.is_empty() { self.drafts.push(ReplyDraft { key: key.clone(), body: body.into() }); }
        true
    }
    fn clear_if_matches(&mut self, key: &ReplyDraftKey, body: &str) -> bool {
        let before = self.drafts.len();
        self.drafts.retain(|draft| &draft.key != key || draft.body != body);
        self.drafts.len() != before
    }
}
fn read_reply_drafts(path: &Path) -> Result<ReplyDrafts> {
    if !path.exists() { return Ok(ReplyDrafts::default()); }
    let drafts: ReplyDrafts = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(drafts.version == 1, "Unsupported Moments reply draft version");
    Ok(drafts)
}
pub fn reply_draft(owner: &ruma::UserId, key: &ReplyDraftKey) -> Result<String> {
    let _lock = REPLY_DRAFT_WRITES.lock().unwrap();
    let path = crate::persistence::persistent_state_dir(owner).join("moments-replies.json");
    Ok(read_reply_drafts(&path)?.get(key))
}
pub fn save_reply_draft(owner: &ruma::UserId, key: &ReplyDraftKey, body: &str) -> Result<()> {
    let _lock = REPLY_DRAFT_WRITES.lock().unwrap();
    let path = crate::persistence::persistent_state_dir(owner).join("moments-replies.json");
    let mut drafts = read_reply_drafts(&path)?;
    if drafts.set(key, body) { write_private(&path, &drafts)?; }
    Ok(())
}
pub fn clear_reply_draft_if_matches(owner: &ruma::UserId, key: &ReplyDraftKey, body: &str) -> Result<()> {
    let _lock = REPLY_DRAFT_WRITES.lock().unwrap();
    let path = crate::persistence::persistent_state_dir(owner).join("moments-replies.json");
    if !path.exists() { return Ok(()); }
    let mut drafts = read_reply_drafts(&path)?;
    if drafts.clear_if_matches(key, body) { write_private(&path, &drafts)?; }
    Ok(())
}
pub fn pending_reply_draft(pending: &Pending) -> Option<(ReplyDraftKey, String)> {
    if pending.event_type != "m.room.message" || pending.content.pointer("/m.relates_to/rel_type")?.as_str()? != "m.thread" {
        return None;
    }
    Some((ReplyDraftKey {
        room: pending.room.clone(),
        post: OwnedEventId::try_from(pending.content.pointer("/m.relates_to/event_id")?.as_str()?).ok()?,
    }, pending.content.get("body")?.as_str()?.to_owned()))
}
#[derive(Clone, Debug)]
pub struct Member {
    pub id: OwnedUserId,
    pub name: String,
    pub invited: bool,
}
#[derive(Clone, Debug)]
pub struct Timeline {
    pub room: OwnedRoomId,
    pub author: OwnedUserId,
    pub name: String,
    pub invited: bool,
    pub members: Vec<Member>,
    pub audience: String,
    pub index: Index,
    pub cursor: Option<String>,
    pub refresh_cursor: Option<String>,
    pub exhausted: bool,
    pub loaded: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Feed {
    pub timelines: BTreeMap<OwnedRoomId, Timeline>,
    pub preferences: Preferences,
    pub next_room: usize,
    pub errors: Vec<String>,
    pub undiscovered: usize,
}
impl Feed {
    pub fn posts(&self, author: Option<&ruma::UserId>) -> Vec<Entry> {
        let mut posts: Vec<_> = self
            .timelines
            .values()
            .filter(|t| {
                !t.invited
                    && author.map_or(!self.preferences.hidden.contains(&t.author), |a| {
                        a == t.author
                    })
            })
            .flat_map(|t| t.index.posts(&t.room, &t.author))
            .collect();
        posts.sort_by(|a, b| (b.timestamp, &b.room, &b.id).cmp(&(a.timestamp, &a.room, &a.id)));
        posts
    }
    pub fn own(&self, owner: &ruma::UserId) -> Vec<&Timeline> {
        self.timelines
            .values()
            .filter(|t| t.author == owner && !t.invited)
            .collect()
    }
    pub fn chosen(&self, owner: &ruma::UserId) -> Option<&Timeline> {
        let own = self.own(owner);
        own.iter()
            .find(|t| Some(&t.room) == self.preferences.timeline.as_ref())
            .copied()
            .or_else(|| (own.len() == 1).then(|| own[0]))
    }
}

#[derive(Clone)]
pub struct Service {
    pub client: Client,
    pub owner: OwnedUserId,
    require_current: bool,
}
impl Service {
    pub fn current() -> Option<Self> {
        let client = crate::sliding_sync::get_client()?;
        let owner = client.user_id()?.to_owned();
        Some(Self {
            client,
            owner,
            require_current: true,
        })
    }
    #[cfg(test)]
    pub fn for_test(client: Client) -> Self {
        Self {
            owner: client.user_id().unwrap().to_owned(),
            client,
            require_current: false,
        }
    }
    fn guard(&self) -> Result<()> {
        ensure!(
            !self.require_current
                || crate::sliding_sync::get_client()
                    .is_some_and(|c| c.user_id() == Some(&self.owner)
                        && c.device_id() == self.client.device_id()),
            crate::i18n::tr("Account changed; operation stopped.")
        );
        Ok(())
    }
    pub async fn preferences(&self) -> Result<Preferences> {
        let raw = self
            .client
            .account()
            .fetch_account_data(ACCOUNT_DATA.into())
            .await?;
        Ok(match raw {
            Some(raw) => serde_json::from_str(raw.json().get())?,
            None => Preferences::default(),
        })
    }
    async fn save_preferences(&self, preferences: &Preferences) -> Result<()> {
        self.guard()?;
        self.client
            .account()
            .set_account_data_raw(ACCOUNT_DATA.into(), Raw::new(preferences)?.cast_unchecked())
            .await?;
        Ok(())
    }
    async fn state(&self, room: &ruma::RoomId) -> Result<Vec<Value>> {
        self.guard()?;
        let response = self
            .client
            .send(get_state_events::v3::Request::new(room.to_owned()))
            .await?;
        response
            .room_state
            .into_iter()
            .map(|r| serde_json::from_str(r.json().get()).map_err(Into::into))
            .collect()
    }
    pub async fn validate(&self, room: &ruma::RoomId) -> Result<Timeline> {
        timeline_from_state(room, &self.state(room).await?)
    }
    async fn writable(&self, room: &ruma::RoomId, author_only: bool) -> Result<(Room, Timeline)> {
        self.guard()?;
        let sdk = self
            .client
            .get_room(room)
            .ok_or_else(|| anyhow::anyhow!(crate::i18n::tr("Waiting for timeline sync. Refresh in a moment.")))?;
        ensure!(
            sdk.state() == RoomState::Joined,
            crate::i18n::tr("Join this timeline first.")
        );
        let timeline = self.validate(room).await?;
        ensure!(
            !author_only || timeline.author == self.owner,
            crate::i18n::tr("Only the author can publish or change this audience.")
        );
        ensure!(
            timeline
                .members
                .iter()
                .any(|m| m.id == self.owner && !m.invited),
            crate::i18n::tr("Your timeline membership has changed.")
        );
        // The local encryption state must be known before send_raw; otherwise it
        // might send plaintext even though the server state is encrypted.
        ensure!(
            sdk.encryption_state().is_encrypted(),
            crate::i18n::tr("Waiting for encryption state to sync. Refresh before sending.")
        );
        Ok((sdk, timeline))
    }
    pub async fn load(&self, mut feed: Feed, older: bool) -> Result<Feed> {
        self.guard()?;
        feed.preferences = self.preferences().await?;
        feed.errors.clear();
        let rooms: Vec<_> = self
            .client
            .rooms()
            .into_iter()
            .filter(|r| {
                super::is_moments(r) && matches!(r.state(), RoomState::Joined | RoomState::Invited)
            })
            .collect();
        // An invitation may be accepted on another device. Drop its stripped
        // metadata so the next discovery pass loads the joined timeline.
        feed.timelines.retain(|id, timeline| {
            rooms.iter().any(|room| {
                room.room_id() == id && timeline.invited == (room.state() == RoomState::Invited)
            })
        });
        let mut discovered = 0;
        for room in &rooms {
            if feed.timelines.contains_key(room.room_id()) {
                continue;
            }
            if discovered == 8 {
                break;
            }
            discovered += 1;
            if room.state() == RoomState::Invited {
                use ruma::events::room::create::RoomCreateEventContent;
                if let Some(raw) = room
                    .get_state_event_static::<RoomCreateEventContent>()
                    .await?
                {
                    let author = raw.deserialize()?.sender().to_owned();
                    {
                        feed.timelines.insert(
                            room.room_id().to_owned(),
                            Timeline {
                                room: room.room_id().to_owned(),
                                name: author.to_string(),
                                author,
                                invited: true,
                                members: vec![],
                                audience: String::new(),
                                index: Index::default(),
                                cursor: None,
                                refresh_cursor: None,
                                exhausted: false,
                                loaded: false,
                            },
                        );
                    }
                }
            } else {
                match self.validate(room.room_id()).await {
                    Ok(t) => {
                        feed.timelines.insert(t.room.clone(), t);
                    }
                    Err(e) => feed.errors.push(crate::i18n::format("Timeline unavailable: {e}", &[("e", (e).to_string())])),
                }
            }
        }
        feed.undiscovered = rooms
            .iter()
            .filter(|r| !feed.timelines.contains_key(r.room_id()))
            .count();
        // Bounded history work per refresh. Each timeline retains its own cursor;
        // a large contact list cannot trigger an unbounded full-history download.
        let ids: Vec<_> = feed.timelines.keys().cloned().collect();
        let start = feed.next_room % ids.len().max(1);
        let mut fetched = 0;
        for offset in 0..ids.len() {
            let id = &ids[(start + offset) % ids.len()];
            let Some(sdk) = self.client.get_room(id) else {
                continue;
            };
            let t = feed.timelines.get_mut(id).unwrap();
            if t.invited || (older && t.exhausted) {
                continue;
            }
            self.guard()?;
            let mut options = MessagesOptions::backward();
            options.limit = ruma::uint!(50);
            if older && t.loaded {
                options.from = t.cursor.clone();
            }
            if !older && t.loaded {
                options.from = t.refresh_cursor.clone();
            }
            match sdk.messages(options).await {
                Ok(page) => {
                    let done = page.end.is_none()
                        || page.chunk.is_empty()
                        || (older && page.end == t.cursor);
                    let reached_loaded = page
                        .chunk
                        .iter()
                        .filter_map(|e| e.kind.parse_event_id())
                        .any(|id| t.index.contains(&id));
                    for event in page.chunk {
                        if let Ok(value) = serde_json::from_str(event.kind.raw().json().get()) {
                            t.index.insert(id, value);
                        }
                    }
                    if !older && t.loaded {
                        t.refresh_cursor = if done || reached_loaded {
                            None
                        } else {
                            page.end.clone()
                        };
                    }
                    if older || !t.loaded {
                        t.cursor = page.end;
                        t.exhausted = done;
                    }
                    t.loaded = true;
                    for id in t.index.unavailable_ids().into_iter().take(4) {
                        if let Ok(event) = sdk.event(&id, None).await {
                            if !event.kind.is_utd() {
                                t.index.insert(
                                    &t.room,
                                    serde_json::from_str(event.kind.raw().json().get())?,
                                );
                            }
                        }
                    }
                }
                Err(e) => feed.errors.push(format!("{}: {e}", t.name)),
            }
            fetched += 1;
            feed.next_room = (start + offset + 1) % ids.len();
            if fetched == 8 {
                break;
            }
        }
        Ok(feed)
    }
    pub async fn ensure_timeline(&self) -> Result<OwnedRoomId> {
        let _lock = WRITES.lock().await;
        self.guard()?;
        let mut prefs = self.preferences().await?;
        let owned = self.reconcile_rooms(true).await?;
        if let Some(id) = prefs.timeline.as_ref().filter(|id| owned.contains(id)) {
            return Ok(id.clone());
        }
        ensure!(
            owned.len() < 2,
            crate::i18n::tr("Multiple My Posts timelines exist. Choose one in Audience; their viewers will remain separate.")
        );
        let id = if let Some(id) = owned.first() {
            id.clone()
        } else {
            self.create_private(true).await?
        };
        prefs.timeline = Some(id.clone());
        self.save_preferences(&prefs).await?;
        Ok(id)
    }
    /// Explicit recovery after an uncertain create. Reconcile again first; a
    /// late original response may still produce a second empty owned timeline,
    /// which the normal duplicate-choice UI keeps separate.
    pub async fn retry_timeline_setup(&self) -> Result<OwnedRoomId> {
        let _lock = WRITES.lock().await;
        self.guard()?;
        let owned = self.reconcile_rooms(true).await?;
        ensure!(
            owned.len() < 2,
            crate::i18n::tr("Choose one of your existing timelines in Audience.")
        );
        let id = if let Some(id) = owned.first() {
            id.clone()
        } else {
            let path = self.path("moments-create.json");
            if path.exists() {
                std::fs::remove_file(path)?;
            }
            self.create_private(true).await?
        };
        let mut prefs = self.preferences().await?;
        prefs.timeline = Some(id.clone());
        self.save_preferences(&prefs).await?;
        Ok(id)
    }
    async fn reconcile_rooms(&self, moments: bool) -> Result<Vec<OwnedRoomId>> {
        use ruma::api::client::membership::joined_rooms;
        // Include rooms missing from local sync after a createRoom timeout.
        let remote = self.client.send(joined_rooms::v3::Request::new()).await?;
        let mut found = vec![];
        for id in remote.joined_rooms {
            self.guard()?;
            if let Some(room) = self.client.get_room(&id) {
                if moments && !super::is_moments(&room) && room.room_type().is_some() {
                    continue;
                }
                // Known untyped chat rooms need no state download for Moments.
                if moments && !super::is_moments(&room) && room.get_state_event_static::<ruma::events::room::create::RoomCreateEventContent>().await?.is_some() {continue;}
            }
            let state = self.state(&id).await?;
            if moments {
                if timeline_from_state(&id, &state).is_ok_and(|t| t.author == self.owner) {
                    found.push(id);
                }
            } else if private_self_state(&state, &self.owner) {
                // Reuse explicit File Transfer rooms or existing self-DMs only.
                let tagged = state.iter().any(|e| {
                    e["type"] == "m.room.create"
                        && e["content"]["rs.robius.robrix.file_transfer"] == true
                });
                let self_dm = if let Some(room) = self.client.get_room(&id) {
                    room.is_direct().await? && room.direct_targets().contains(self.owner.as_str())
                } else {
                    false
                };
                if tagged || self_dm {
                    found.push(id);
                }
            }
        }
        Ok(found)
    }
    async fn create_private(&self, moments: bool) -> Result<OwnedRoomId> {
        let filename = if moments {
            "moments-create.json"
        } else {
            "file-transfer-create.json"
        };
        let path = self.path(filename);
        // A previous uncertain create is reconciled by ensure_* before arriving
        // here. Do not issue another create and risk splitting its audience.
        if path.exists() {
            bail!(
                crate::i18n::tr("A room creation is awaiting sync. Refresh and retry after it appears; no duplicate room was created.")
            );
        }
        let operation = TransactionId::new();
        write_private(&path, &json!({"operation":operation}))?;
        let mut request = create_room::v3::Request::new();
        request.name = Some(
            if moments {
                "My Moments"
            } else {
                crate::i18n::tr("File Transfer")
            }
            .into(),
        );
        request.preset = Some(create_room::v3::RoomPreset::PrivateChat);
        request.creation_content=Some(Raw::new(&if moments {json!({"type":ROOM_TYPE,"rs.robius.robrix.operation":operation})}
            else {json!({"rs.robius.robrix.file_transfer":true,"rs.robius.robrix.operation":operation})})?.cast_unchecked());
        request.initial_state=vec![
            json!({"type":"m.room.encryption","state_key":"","content":{"algorithm":"m.megolm.v1.aes-sha2"}}),
            json!({"type":"m.room.history_visibility","state_key":"","content":{"history_visibility":"joined"}}),
            json!({"type":"m.room.join_rules","state_key":"","content":{"join_rule":"invite"}}),
        ].iter().map(|v|Raw::new(v).map(Raw::cast_unchecked)).collect::<Result<_,_>>()?;
        // No "users" entry for the owner: the server's default already gives the creator
        // full power, and room version 12 (matrix.org's default) rejects listing a creator.
        request.power_level_content_override=Some(Raw::new(&json!({"users_default":0,
            "events_default":0,"state_default":100,"invite":100,"kick":100,"ban":100,"redact":100,
            "events":{"m.room.name":100,"m.room.topic":100,"m.room.avatar":100,"m.room.power_levels":100,"m.room.join_rules":100,"m.room.history_visibility":100,"m.room.encryption":100}}))?.cast_unchecked());
        self.guard()?;
        let room = match self.client.create_room(request).await {
            Ok(room) => room,
            Err(e) => {
                // A definitive Matrix error cannot have created a room. A network
                // timeout is uncertain and retains the recovery intent.
                if e.client_api_error_kind().is_some() {
                    let _ = std::fs::remove_file(&path);
                }
                return Err(e.into());
            }
        };
        let id = room.room_id().to_owned();
        write_private(&path, &json!({"operation":operation,"room_id":id}))?;
        Ok(id)
    }
    pub async fn file_transfer(&self) -> Result<OwnedRoomId> {
        let _lock = WRITES.lock().await;
        self.guard()?;
        let mut prefs = self.preferences().await?;
        if let Some(id) = &prefs.file_transfer {
            if private_self_state(&self.state(id).await?, &self.owner) {
                return Ok(id.clone());
            }
            bail!(
                crate::i18n::tr("Your previous File Transfer has other members or changed privacy. Choose New Private File Transfer to keep the old conversation separate.")
            );
        }
        let existing = self.reconcile_rooms(false).await?;
        let id = match existing.first() {
            Some(id) => id.clone(),
            None => self.create_private(false).await?,
        };
        prefs.file_transfer = Some(id.clone());
        self.save_preferences(&prefs).await?;
        Ok(id)
    }
    pub async fn new_file_transfer(&self) -> Result<OwnedRoomId> {
        let _lock = WRITES.lock().await;
        self.guard()?;
        let path = self.path("file-transfer-create.json");
        // Only a confirmed previous operation can be replaced intentionally.
        if path.exists() {
            let v: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
            ensure!(
                v["room_id"].is_string(),
                crate::i18n::tr("Previous creation is still uncertain. Refresh first.")
            );
            std::fs::remove_file(path)?;
        }
        let id = self.create_private(false).await?;
        let mut prefs = self.preferences().await?;
        prefs.file_transfer = Some(id.clone());
        self.save_preferences(&prefs).await?;
        Ok(id)
    }
    pub async fn choose(&self, room: OwnedRoomId) -> Result<()> {
        let _lock = WRITES.lock().await;
        self.writable(&room, true).await?;
        let mut prefs = self.preferences().await?;
        prefs.timeline = Some(room);
        self.save_preferences(&prefs).await
    }
    /// Applies `update` to the saved preferences.
    pub(super) async fn update_preferences(&self, update: impl FnOnce(&mut Preferences)) -> Result<()> {
        let _lock = WRITES.lock().await;
        let mut prefs = self.preferences().await?;
        update(&mut prefs);
        self.save_preferences(&prefs).await
    }
    pub async fn hide(&self, author: OwnedUserId, hidden: bool) -> Result<()> {
        let _lock = WRITES.lock().await;
        let mut prefs = self.preferences().await?;
        if hidden {
            prefs.hidden.insert(author);
        } else {
            prefs.hidden.remove(&author);
        }
        self.save_preferences(&prefs).await
    }
    pub async fn mark_seen(&self, ids: Vec<OwnedEventId>) -> Result<()> {
        let _lock = WRITES.lock().await;
        let mut prefs = self.preferences().await?;
        prefs.seen.extend(ids);
        while prefs.seen.len() > 1000 {
            prefs.seen.pop_first();
        }
        self.save_preferences(&prefs).await
    }
    pub async fn membership(
        &self,
        room: &ruma::RoomId,
        user: &ruma::UserId,
        invite: bool,
    ) -> Result<()> {
        let _lock = WRITES.lock().await;
        let (sdk, _) = self.writable(room, true).await?;
        ensure!(user != self.owner, crate::i18n::tr("The author cannot be removed."));
        self.guard()?;
        if invite {
            sdk.invite_user_by_id(user).await?;
        } else {
            sdk.kick_user(user, Some("Removed from Moments audience"))
                .await?;
            // Confirm server membership AND wait for the SDK membership update;
            // otherwise key sharing could still use an obsolete member list.
            for _ in 0..30 {
                self.guard()?;
                let state = self.validate(room).await?;
                let members = sdk.members(matrix_sdk::RoomMemberships::ACTIVE).await?;
                if !state.members.iter().any(|m| m.id == user)
                    && !members.iter().any(|m| m.user_id() == user)
                {
                    sdk.discard_room_key().await?;
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
            bail!(crate::i18n::tr("Removal is awaiting sync. Publishing is paused until membership updates."));
        }
        Ok(())
    }
    pub async fn invitation(&self, room: &ruma::RoomId, accept: bool) -> Result<()> {
        let _lock = WRITES.lock().await;
        self.guard()?;
        let sdk = self
            .client
            .get_room(room)
            .ok_or_else(|| anyhow::anyhow!(crate::i18n::tr("Invitation unavailable")))?;
        ensure!(super::is_moments(&sdk), crate::i18n::tr("This is not a Moments invitation."));
        if accept {
            sdk.join().await?;
        } else {
            sdk.leave().await?;
        }
        Ok(())
    }
    pub fn path(&self, name: &str) -> PathBuf {
        crate::persistence::persistent_state_dir(&self.owner).join(name)
    }
    pub fn pending(&self) -> Result<Option<Pending>> {
        let path = self.path("moments-outbox.json");
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&std::fs::read(path)?)?))
    }
    fn cleanup_sent_post(&self, pending: &Pending) {
        let draft_path = self.path("moments-composer.json");
        let draft = std::fs::read(&draft_path).ok()
            .and_then(|bytes| serde_json::from_slice::<ComposerDraft>(&bytes).ok());
        let matches_sent = draft.as_ref().is_some_and(|draft| {
            draft.paths == pending.paths
                && pending.draft_body.as_deref().map_or_else(
                    || draft.body == pending.content["body"].as_str().unwrap_or("")
                        || (draft.body.is_empty() && !draft.paths.is_empty()),
                    |original| draft.body == original,
                )
        });
        if matches_sent {
            let _ = std::fs::remove_file(&draft_path);
        }
        let retained_paths = draft.as_ref().map(|draft| &draft.paths);
        let staged = self.path("moments-drafts");
        for path in &pending.paths {
            if path.starts_with(&staged)
                && (!draft_path.exists()
                    || retained_paths.is_some_and(|paths| !paths.contains(path))) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    pub async fn send(&self, mut pending: Pending) -> Result<OwnedEventId> {
        let _lock = WRITES.lock().await;
        self.guard()?;
        if let Some(previous) = self.pending()? {
            if previous.transaction != pending.transaction {
                ensure!(
                    previous.confirmed.is_some(),
                    crate::i18n::tr("A previous post or comment is pending. Retry it first.")
                );
            } else {
                pending = previous;
            }
        }
        if let Some(id) = pending.confirmed.clone() {
            if pending.is_post { self.cleanup_sent_post(&pending); }
            return Ok(id);
        }
        ensure!(
            pending.paths.len() <= MAX_MEDIA,
            crate::i18n::tr("Choose at most nine photos or videos.")
        );
        ensure!(
            pending.content["body"].as_str().unwrap_or("").len() <= MAX_TEXT,
            crate::i18n::tr("Post is too long.")
        );
        self.save_pending(&pending)?;
        let (sdk, timeline) = self.writable(&pending.room, pending.is_post).await?;
        ensure!(
            pending.audience == timeline.audience,
            crate::i18n::tr("Timeline audience changed. Review Audience before retrying the saved draft.")
        );
        for path in pending.paths.iter().skip(pending.assets.len()) {
            self.guard()?;
            let size = std::fs::metadata(path)?.len();
            ensure!(
                size <= 25 * 1024 * 1024,
                crate::i18n::tr("Each attachment must be at most 25 MB.")
            );
            let mime = mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string();
            ensure!(
                mime.starts_with("image/") || mime.starts_with("video/"),
                crate::i18n::tr("Choose an image or video.")
            );
            let mut file = std::fs::File::open(path)?;
            let encrypted = self.client.upload_encrypted_file(&mut file).await?;
            pending.assets.push(Asset {
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                mimetype: mime,
                size,
                file: encrypted,
            });
            self.save_pending(&pending)?;
        }
        if pending.is_post {
            pending.content = super::model::post_content(
                pending.content["body"].as_str().unwrap_or(""),
                &pending.assets,
            );
        }
        let (_, fresh) = self.writable(&pending.room, pending.is_post).await?;
        ensure!(
            pending.audience == fresh.audience,
            crate::i18n::tr("Audience changed during upload. Review Audience before retrying.")
        );
        // Confirm the SDK's active recipients agree with the authoritative state
        // before it shares keys. Rotate on every author post for an explicit cut.
        let cached: BTreeSet<_> = sdk
            .members(matrix_sdk::RoomMemberships::ACTIVE)
            .await?
            .into_iter()
            .map(|m| m.user_id().to_owned())
            .collect();
        let expected: BTreeSet<_> = fresh.members.iter().map(|m| m.id.clone()).collect();
        ensure!(
            cached == expected,
            crate::i18n::tr("Membership is still syncing. Refresh and retry.")
        );
        if pending.is_post {
            sdk.discard_room_key().await?;
        }
        self.guard()?;
        self.save_pending(&pending)?;
        let sent = sdk
            .send_raw(&pending.event_type, pending.content.clone())
            .with_transaction_id(&pending.transaction)
            .await;
        let sent = match sent {
            Ok(sent) => sent,
            Err(error)
                if pending.event_type == "m.reaction"
                    && error.client_api_error_kind()
                        == Some(&ruma::api::error::ErrorKind::DuplicateAnnotation) =>
            {
                // Synapse rejects a second annotation from another device.
                // Reconcile it to the existing event instead of trapping the
                // account behind an impossible outbox retry.
                let target = OwnedEventId::try_from(
                    pending.content["m.relates_to"]["event_id"]
                        .as_str()
                        .unwrap_or(""),
                )?;
                let _ = std::fs::remove_file(self.path("moments-outbox.json"));
                if let Some(id) = self.existing_like(&sdk, &target).await? {
                    pending.confirmed = Some(id.clone());
                    self.save_pending(&pending)?;
                    return Ok(id);
                }
                bail!(crate::i18n::tr("This post is already liked. Refresh to update its reactions."));
            }
            Err(error) => return Err(error.into()),
        };
        pending.confirmed = Some(sent.response.event_id.clone());
        self.save_pending(&pending)?;
        if pending.is_post { self.cleanup_sent_post(&pending); }
        Ok(sent.response.event_id)
    }
    pub fn save_pending(&self, pending: &Pending) -> Result<()> {
        write_private(&self.path("moments-outbox.json"), pending)
    }
    async fn existing_like(
        &self,
        room: &Room,
        target: &ruma::EventId,
    ) -> Result<Option<OwnedEventId>> {
        let mut cursor = None;
        for _ in 0..5 {
            self.guard()?;
            let page = room
                .relations(
                    target.to_owned(),
                    matrix_sdk::room::RelationsOptions {
                        from: cursor.clone(),
                        limit: Some(ruma::uint!(100)),
                        include_relations:
                            matrix_sdk::room::IncludeRelations::RelationsOfTypeAndEventType(
                                ruma::events::relation::RelationType::Annotation,
                                ruma::events::TimelineEventType::Reaction,
                            ),
                        ..Default::default()
                    },
                )
                .await?;
            for event in page.chunk {
                let value: Value = serde_json::from_str(event.kind.raw().json().get())?;
                if value["sender"] == self.owner.as_str()
                    && value["content"]["m.relates_to"]["key"] == LIKE
                    && value.pointer("/unsigned/redacted_because").is_none()
                {
                    return Ok(Some(OwnedEventId::try_from(
                        value["event_id"].as_str().unwrap_or(""),
                    )?));
                }
            }
            if page.prev_batch_token.is_none() || page.prev_batch_token == cursor {
                break;
            }
            cursor = page.prev_batch_token;
        }
        Ok(None)
    }
    pub async fn discard_pending(&self) -> Result<()> {
        let _lock = WRITES.lock().await;
        self.guard()?;
        let path = self.path("moments-outbox.json");
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
    pub async fn review_pending(&self, room: &ruma::RoomId, shown_audience: &str) -> Result<()> {
        let _lock = WRITES.lock().await;
        if let Some(mut pending) = self.pending()? {
            ensure!(
                pending.room == room,
                crate::i18n::tr("This saved send belongs to a different timeline. Reopen its audience.")
            );
            let (_, timeline) = self.writable(&pending.room, pending.is_post).await?;
            ensure!(
                timeline.audience == shown_audience,
                crate::i18n::tr("The audience changed again. Reopen Audience to review the current viewers.")
            );
            pending.audience = timeline.audience;
            self.save_pending(&pending)?;
        }
        Ok(())
    }
    pub async fn redact(&self, room: &ruma::RoomId, events: Vec<OwnedEventId>) -> Result<()> {
        let _lock = WRITES.lock().await;
        let (sdk, _) = self.writable(room, false).await?;
        for id in events {
            self.guard()?;
            sdk.redact(&id, None, Some(TransactionId::new())).await?;
        }
        Ok(())
    }
    pub async fn interact(
        &self,
        post: &Entry,
        content: Value,
        event_type: &str,
        txn: OwnedTransactionId,
    ) -> Result<OwnedEventId> {
        // Re-fetch the target: a cached row alone is insufficient to authorize a
        // relationship to a post that may have been redacted in the meantime.
        let (sdk, timeline) = self.writable(&post.room, false).await?;
        let target = sdk.event(&post.id, None).await?;
        let mut index = Index::default();
        index.insert(
            &post.room,
            serde_json::from_str(target.kind.raw().json().get())?,
        );
        ensure!(
            index
                .posts(&post.room, &timeline.author)
                .iter()
                .any(|p| p.id == post.id),
            crate::i18n::tr("This post is no longer available.")
        );
        self.send(Pending {
            transaction: txn,
            room: post.room.clone(),
            audience: timeline.audience,
            event_type: event_type.into(),
            content,
            is_post: false,
            draft_body: None,
            paths: vec![],
            assets: vec![],
            confirmed: None,
        })
        .await
    }
    pub async fn like(&self, post: &Entry, txn: OwnedTransactionId) -> Result<OwnedEventId> {
        self.interact(
            post,
            json!({"m.relates_to":{"rel_type":"m.annotation","event_id":post.id,"key":LIKE}}),
            "m.reaction",
            txn,
        )
        .await
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pending {
    pub transaction: OwnedTransactionId,
    pub room: OwnedRoomId,
    pub audience: String,
    pub event_type: String,
    pub content: Value,
    pub is_post: bool,
    #[serde(default)]
    pub draft_body: Option<String>,
    pub paths: Vec<PathBuf>,
    pub assets: Vec<Asset>,
    pub confirmed: Option<OwnedEventId>,
}

fn state_content<'a>(state: &'a [Value], kind: &str) -> Option<&'a Value> {
    state
        .iter()
        .find(|e| e["type"] == kind && e["state_key"] == "")
        .map(|e| &e["content"])
}
pub fn timeline_from_state(room: &ruma::RoomId, state: &[Value]) -> Result<Timeline> {
    let create = state
        .iter()
        .find(|e| e["type"] == "m.room.create" && e["state_key"] == "")
        .ok_or_else(|| anyhow::anyhow!(crate::i18n::tr("Room creation unavailable")))?;
    ensure!(
        create["content"]["type"] == ROOM_TYPE,
        crate::i18n::tr("Not a Moments timeline.")
    );
    ensure!(
        create["content"]["additional_creators"]
            .as_array()
            .is_none_or(|a| a.is_empty()),
        crate::i18n::tr("Shared-ownership timelines are unsupported.")
    );
    let author = OwnedUserId::try_from(create["sender"].as_str().unwrap_or(""))?;
    ensure!(
        state_content(state, "m.room.encryption")
            .is_some_and(|c| c["algorithm"] == "m.megolm.v1.aes-sha2"),
        crate::i18n::tr("Timeline encryption is unavailable.")
    );
    ensure!(
        state_content(state, "m.room.join_rules").is_some_and(|c| c["join_rule"] == "invite"),
        crate::i18n::tr("Timeline must be invite-only.")
    );
    ensure!(
        state_content(state, "m.room.history_visibility")
            .is_some_and(|c| c["history_visibility"] == "joined"),
        crate::i18n::tr("Timeline history must be limited to joined viewers.")
    );
    let powers = state_content(state, "m.room.power_levels")
        .ok_or_else(|| anyhow::anyhow!(crate::i18n::tr("Timeline permissions unavailable")))?;
    ensure!(
        powers["invite"].as_i64().unwrap_or(0) >= 100
            && powers["state_default"].as_i64().unwrap_or(50) >= 100
            && powers["kick"].as_i64().unwrap_or(50) >= 100
            && powers["ban"].as_i64().unwrap_or(50) >= 100
            && powers["users_default"].as_i64().unwrap_or(0) < 100,
        crate::i18n::tr("Timeline audience permissions changed.")
    );
    for kind in [
        "m.room.power_levels",
        "m.room.join_rules",
        "m.room.history_visibility",
        "m.room.encryption",
        "m.room.name",
    ] {
        ensure!(
            powers["events"][kind].as_i64().unwrap_or(100) >= 100,
            crate::i18n::tr("Only the author may change timeline settings.")
        );
    }
    // Room version 12 creators hold full power implicitly and may have no `users` entries at all.
    ensure!(
        powers["users"].as_object().is_none_or(|users| users
            .iter()
            .all(|(u, p)| u == author.as_str() || p.as_i64().unwrap_or(100) < 100)),
        crate::i18n::tr("Only the author may manage this audience.")
    );
    let mut members = vec![];
    let mut audience = vec![];
    for event in state.iter().filter(|e| e["type"] == "m.room.member") {
        if !matches!(
            event["content"]["membership"].as_str(),
            Some("join" | "invite")
        ) {
            continue;
        }
        let id = OwnedUserId::try_from(event["state_key"].as_str().unwrap_or(""))?;
        let invited = event["content"]["membership"] == "invite";
        let name = event["content"]["displayname"]
            .as_str()
            .unwrap_or(id.as_str())
            .to_string();
        audience.push(format!("{id}:{}", if invited { "invite" } else { "join" }));
        members.push(Member { id, name, invited });
    }
    audience.sort();
    members.sort_by(|a, b| a.id.cmp(&b.id));
    let name = members
        .iter()
        .find(|m| m.id == author)
        .map(|m| m.name.clone())
        .unwrap_or_else(|| author.to_string());
    Ok(Timeline {
        room: room.to_owned(),
        author,
        name,
        invited: false,
        members,
        audience: audience.join("\n"),
        index: Index::default(),
        cursor: None,
        refresh_cursor: None,
        exhausted: false,
        loaded: false,
    })
}
fn private_self_state(state: &[Value], owner: &ruma::UserId) -> bool {
    state_content(state, "m.room.create").is_some_and(|c| c.get("type").is_none())
        && state_content(state, "m.room.encryption")
            .is_some_and(|c| c["algorithm"] == "m.megolm.v1.aes-sha2")
        && state_content(state, "m.room.join_rules").is_some_and(|c| c["join_rule"] == "invite")
        && state_content(state, "m.room.history_visibility")
            .is_some_and(|c| c["history_visibility"] == "joined")
        && state.iter().any(|e| {
            e["type"] == "m.room.member"
                && e["state_key"] == owner.as_str()
                && e["content"]["membership"] == "join"
        })
        && state
            .iter()
            .filter(|e| {
                e["type"] == "m.room.member"
                    && matches!(
                        e["content"]["membership"].as_str(),
                        Some("join" | "invite" | "knock")
                    )
            })
            .all(|e| e["state_key"] == owner.as_str())
}
pub fn write_private(path: &Path, value: &impl Serialize) -> Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reply_drafts_are_per_post_and_clear_only_the_sent_revision() {
        let first = ReplyDraftKey {
            room: ruma::room_id!("!room:example.org").to_owned(),
            post: ruma::event_id!("$first:example.org").to_owned(),
        };
        let second = ReplyDraftKey {
            room: first.room.clone(),
            post: ruma::event_id!("$second:example.org").to_owned(),
        };
        let other_room = ReplyDraftKey {
            room: ruma::room_id!("!other:example.org").to_owned(),
            post: first.post.clone(),
        };
        let mut drafts = ReplyDrafts::default();
        drafts.set(&first, "first reply");
        drafts.set(&second, "second reply");
        drafts.set(&other_room, "other room reply");
        let encoded = serde_json::to_vec(&drafts).unwrap();
        let mut restored: ReplyDrafts = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored.get(&first), "first reply");
        assert_eq!(restored.get(&second), "second reply");
        assert_eq!(restored.get(&other_room), "other room reply");

        restored.set(&first, "revised while sending");
        assert!(!restored.clear_if_matches(&first, "first reply"));
        assert_eq!(restored.get(&first), "revised while sending");
        assert!(restored.clear_if_matches(&second, "second reply"));
        assert!(restored.get(&second).is_empty());
        assert_eq!(restored.get(&other_room), "other room reply");
    }
    #[test]
    fn only_thread_replies_map_to_reply_drafts() {
        let room = ruma::room_id!("!room:example.org").to_owned();
        let post = ruma::event_id!("$post:example.org").to_owned();
        let mut pending = Pending {
            transaction: TransactionId::new(), room: room.clone(), audience: String::new(),
            event_type: "m.room.message".into(),
            content: super::super::model::comment_content("reply", &post),
            is_post: false, draft_body: None, paths: vec![], assets: vec![], confirmed: None,
        };
        assert_eq!(pending_reply_draft(&pending), Some((ReplyDraftKey { room, post }, "reply".into())));
        pending.content["m.relates_to"]["rel_type"] = json!("m.replace");
        assert!(pending_reply_draft(&pending).is_none());
    }
    fn state() -> Vec<Value> {
        vec![
            json!({"type":"m.room.create","state_key":"","sender":"@author:example.org","content":{"type":ROOM_TYPE}}),
            json!({"type":"m.room.encryption","state_key":"","content":{"algorithm":"m.megolm.v1.aes-sha2"}}),
            json!({"type":"m.room.join_rules","state_key":"","content":{"join_rule":"invite"}}),
            json!({"type":"m.room.history_visibility","state_key":"","content":{"history_visibility":"joined"}}),
            json!({"type":"m.room.power_levels","state_key":"","content":{"invite":100,"kick":100,"ban":100,"state_default":100,"users_default":0,"users":{"@author:example.org":100}}}),
            json!({"type":"m.room.member","state_key":"@author:example.org","content":{"membership":"join","displayname":"Author"}}),
        ]
    }
    #[test]
    fn room_identity_and_audience_are_validated_before_sending() {
        let id = ruma::room_id!("!moments:example.org");
        let state = state();
        let timeline = timeline_from_state(id, &state).unwrap();
        assert_eq!(timeline.author, ruma::user_id!("@author:example.org"));
        for (index, key, value) in [
            (0, "type", json!("m.space")),
            (1, "algorithm", json!("plaintext")),
            (2, "join_rule", json!("public")),
            (3, "history_visibility", json!("shared")),
            (4, "invite", json!(0)),
            (4, "kick", json!(0)),
        ] {
            let mut bad = state.clone();
            bad[index]["content"][key] = value;
            assert!(timeline_from_state(id, &bad).is_err());
        }
        let mut elevated = state.clone();
        elevated[4]["content"]["events"] = json!({"m.room.power_levels":0});
        assert!(timeline_from_state(id, &elevated).is_err());
        // Room version 12: the creator's power is implicit, so `users` may be empty or absent.
        let mut v12 = state.clone();
        v12[4]["content"]["users"] = json!({});
        assert!(timeline_from_state(id, &v12).is_ok());
        v12[4]["content"].as_object_mut().unwrap().remove("users");
        assert!(timeline_from_state(id, &v12).is_ok());
        v12[4]["content"]["users"] = json!({"@viewer:example.org":100});
        assert!(timeline_from_state(id, &v12).is_err());
        let mut extra_creator = state.clone();
        extra_creator[0]["content"]["additional_creators"] = json!(["@viewer:example.org"]);
        assert!(timeline_from_state(id, &extra_creator).is_err());
        let mut named = state.clone();
        named.push(json!({"type":"m.room.name","state_key":"","content":{"name":"@someone_else:example.org"}}));
        assert_eq!(
            timeline_from_state(id, &named).unwrap().author,
            timeline.author
        );
        let mut joined = state;
        joined.push(json!({"type":"m.room.member","state_key":"@viewer:example.org","content":{"membership":"invite"}}));
        assert_ne!(
            timeline_from_state(id, &joined).unwrap().audience,
            timeline.audience
        );
    }
    #[test]
    fn sharing_with_dm_contacts_is_on_unless_turned_off() {
        assert!(Preferences::default().share_with_dm_contacts);
        // Preferences saved before the setting existed.
        let old: Preferences = serde_json::from_value(json!({"timeline":null,"file_transfer":null})).unwrap();
        assert!(old.share_with_dm_contacts);
        let off: Preferences = serde_json::from_value(json!({"timeline":null,"file_transfer":null,"share_with_dm_contacts":false})).unwrap();
        assert!(!off.share_with_dm_contacts);
    }
    #[test]
    fn file_transfer_rejects_moments_and_other_members_even_pending() {
        let owner = ruma::user_id!("@author:example.org");
        let mut state = state();
        assert!(!private_self_state(&state, owner));
        state[0]["content"] = json!({});
        assert!(private_self_state(&state, owner));
        for membership in ["join", "invite", "knock"] {
            let mut unsafe_room = state.clone();
            unsafe_room.push(json!({"type":"m.room.member","state_key":"@other:example.org","content":{"membership":membership}}));
            assert!(!private_self_state(&unsafe_room, owner));
        }
        state[3]["content"]["history_visibility"] = json!("world_readable");
        assert!(!private_self_state(&state, owner));
    }
    #[test]
    fn hidden_authors_and_duplicate_timeline_choices_are_local_presentation() {
        let owner = ruma::user_id!("@author:example.org");
        let mut feed = Feed::default();
        for room in [
            ruma::room_id!("!one:example.org"),
            ruma::room_id!("!two:example.org"),
        ] {
            let mut t = timeline_from_state(room, &state()).unwrap();
            t.index.insert(room,json!({"event_id":"$same-local-id","type":"m.room.message","sender":owner,"origin_server_ts":1,"content":super::super::model::post_content("Post",&[])}));
            feed.timelines.insert(room.to_owned(), t);
        }
        assert!(feed.chosen(owner).is_none());
        assert_eq!(feed.posts(None).len(), 2);
        feed.preferences.timeline = Some(ruma::room_id!("!two:example.org").to_owned());
        assert_eq!(
            feed.chosen(owner).unwrap().room,
            ruma::room_id!("!two:example.org")
        );
        feed.preferences.hidden.insert(owner.to_owned());
        assert!(feed.posts(None).is_empty());
        assert_eq!(feed.posts(Some(owner)).len(), 2);
    }
}
