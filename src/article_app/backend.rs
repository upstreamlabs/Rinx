//! Matrix article operations. The host owns identity, media and confirmation.
use std::collections::{BTreeMap, BTreeSet};
use article_core::host::Capability;
use matrix_sdk::{
    Client, Room, RoomState,
    config::RequestConfig,
    room::{RelationsOptions, IncludeRelations},
};
use ruma::{
    OwnedEventId, OwnedRoomId,
    events::{relation::RelationType, room::MediaSource},
};
use serde::{Serialize, Deserialize};
use serde_json::{json, Value};
use super::{
    document::*,
    storage::{self, Operation, OperationKind, Publication, RemoteAsset},
    model::Grant,
};
pub const ARTICLE_KEY: &str = "org.octosense.article";
// Matrix caps the complete federated event at 65,536 bytes. Megolm expands
// plaintext by roughly 4/3, in addition to its envelope, padding and signatures.
// Keep room for those and the server's event metadata; 58 KB of plaintext does
// not fit once encrypted. Measure serialized UTF-8 JSON, including retry IDs.
const ENCRYPTED_CONTENT_LIMIT: usize = 44 * 1024;
const PLAIN_CONTENT_LIMIT: usize = 58_000;
const ARTICLE_DATA_LIMIT: usize = 55_000;
static WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArticleContent {
    pub schema: u32,
    pub version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    #[serde(serialize_with = "serialize_wire_document")]
    pub document: Document,
    pub assets: BTreeMap<String, RemoteAsset>,
}
impl ArticleContent {
    pub fn parse(content: &Value) -> Result<Self, String> {
        let value = content
            .get("m.new_content")
            .unwrap_or(content)
            .get(ARTICLE_KEY)
            .ok_or("Not a Rinx article")?;
        if serde_json::to_vec(value).map_err(|e| e.to_string())?.len() > ARTICLE_DATA_LIMIT {
            return Err("Article is too large".into());
        }
        let article: Self =
            serde_json::from_value(value.clone()).map_err(|_| "Invalid article data")?;
        if article.schema != 2
            || article.version == 0
            || article.assets.len() > MAX_IMAGES
            || article
                .transaction_id
                .as_ref()
                .is_some_and(|id| !valid_id(id))
        {
            return Err("Unsupported article version".into());
        }
        article.document.ready()?;
        if article.assets.keys().cloned().collect::<Vec<_>>() != article.document.asset_ids() {
            return Err("Invalid article image".into());
        }
        for (id, remote) in &article.assets {
            let a = &remote.asset;
            let uri = match &remote.source {
                MediaSource::Plain(uri) => uri,
                MediaSource::Encrypted(file) => &file.url,
            };
            if a.id != *id
                || id.len() != 64
                || !id.bytes().all(|c| c.is_ascii_hexdigit())
                || a.bytes == 0
                || a.bytes > storage::MAX_FILE
                || a.mime != "image/png"
                || a.width == 0
                || a.height == 0
                || a.width > 2048
                || a.height > 2048
                || uri.parts().is_err()
            {
                return Err("Invalid article image".into());
            }
        }
        for id in article.document.asset_ids() {
            if !article.assets.contains_key(&id) {
                return Err("Article image is missing".into());
            }
        }
        Ok(article)
    }
}
/// Omit only fields that schema 2 readers already restore from serde defaults.
/// This is a wire optimization: changing Document/Block serialization itself
/// would invalidate the digest used to retain a draft's original Markdown.
fn serialize_wire_document<S: serde::Serializer>(doc: &Document, serializer: S) -> Result<S::Ok, S::Error> {
    let mut value = serde_json::to_value(doc).map_err(serde::ser::Error::custom)?;
    if let Some(blocks) = value["blocks"].as_array_mut() {
        for block in blocks {
            if let Some(marks) = block["marks"].as_array_mut() {
                for mark in marks {
                    if let Some(fields) = mark.as_object_mut() {
                        fields.retain(|key, value| !match key.as_str() {
                            "bold" | "italic" => value.as_bool() == Some(false),
                            "link" => value.is_null(),
                            _ => false,
                        });
                    }
                }
            }
            if let Some(fields) = block.as_object_mut() {
                fields.retain(|key, value| !match key.as_str() {
                    "text" | "caption" | "alt" => value.as_str() == Some(""),
                    "marks" => value.as_array().is_some_and(Vec::is_empty),
                    "asset" => value.is_null(),
                    "width" => value.as_u64() == Some(100),
                    _ => false,
                });
            }
        }
    }
    value.serialize(serializer)
}

fn content_size(content: &Value) -> Result<usize, String> {
    serde_json::to_vec(content).map(|bytes| bytes.len()).map_err(|e| e.to_string())
}

fn remove_html_fallback(content: &mut Value) {
    if let Some(fields) = content.as_object_mut() {
        fields.remove("format");
        fields.remove("formatted_body");
    }
}

pub fn wire_content(
    doc: &Document,
    version: u64,
    assets: &BTreeMap<String, RemoteAsset>,
    root: Option<&ruma::EventId>,
    transaction_id: Option<&str>,
    encrypted: bool,
) -> Result<Value, String> {
    doc.ready()?;
    if transaction_id.is_some_and(|id| !valid_id(id)) {
        return Err("Invalid publication transaction".into());
    }
    let mut html = format!(
        "<h1>{}</h1><p>{}</p>",
        escape(&doc.title),
        escape(&doc.author)
    );
    if let Some(cover) = &doc.cover {
        if cover.show_in_article {
            if let Some(RemoteAsset {
                source: MediaSource::Plain(uri),
                ..
            }) = assets.get(&cover.asset)
            {
                html.push_str(&format!(
                    "<p><img src=\"{}\" alt=\"{}\"></p>",
                    escape(uri.as_str()),
                    escape(&doc.title)
                ));
            }
        }
    }
    for b in &doc.blocks {
        if b.kind == BlockKind::Image {
            let remote = assets
                .get(b.asset.as_deref().unwrap_or(""))
                .ok_or("Article image is missing")?;
            if let MediaSource::Plain(uri) = &remote.source {
                html.push_str(&format!(
                    "<p><img src=\"{}\" alt=\"{}\"></p>",
                    escape(uri.as_str()),
                    escape(&b.alt)
                ));
            }
            html.push_str(&format!(
                "<p>{}</p>",
                escape(if b.caption.is_empty() {
                    &b.alt
                } else {
                    &b.caption
                })
            ));
        } else {
            html.push_str(&doc.block_html(b))
        }
    }
    let plain = crate::shared::slash_commands::html_to_plaintext(&html);
    let article = ArticleContent {
        schema: 2,
        version,
        transaction_id: transaction_id.map(str::to_owned),
        document: doc.clone(),
        assets: assets.clone(),
    };
    let mut content = json!({"msgtype":"m.text","body":plain,"format":"org.matrix.custom.html","formatted_body":html,"m.mentions":{},ARTICLE_KEY:article});
    if content_size(&content[ARTICLE_KEY])? > ARTICLE_DATA_LIMIT {
        return Err("This article is too large to send. Shorten it and try again.".into());
    }
    if let Some(root) = root {
        let new = content.clone();
        // Only m.new_content carries the authoritative article on an edit.
        // Legacy clients still get the complete text/HTML fallback outside it.
        content.as_object_mut().unwrap().remove(ARTICLE_KEY);
        content["body"] = json!(format!("* {}", content["body"].as_str().unwrap_or("")));
        content["formatted_body"] = json!(format!("* {}", content["formatted_body"].as_str().unwrap_or("")));
        content["m.new_content"] = new;
        content["m.relates_to"] = json!({"rel_type":"m.replace","event_id":root});
    }
    let limit = if encrypted { ENCRYPTED_CONTENT_LIMIT } else { PLAIN_CONTENT_LIMIT };
    if content_size(&content)? > limit {
        // Generated HTML is optional in Matrix. Keep the complete plain text
        // and lossless native document instead of truncating the user's post.
        remove_html_fallback(&mut content);
    }
    if content_size(&content)? > limit {
        if let Some(new) = content.get_mut("m.new_content") {
            remove_html_fallback(new);
        }
    }
    if content_size(&content)? > limit {
        return Err("This article is too large to send. Shorten it and try again.".into());
    }
    Ok(content)
}
fn guard(client: &Client, grant: &Grant) -> Result<(), String> {
    if !grant.valid(client.user_id())
        || !grant.valid(crate::sliding_sync::current_user_id().as_deref())
        || crate::logout::logout_state_machine::is_logout_in_progress()
    {
        Err("Authorization expired".into())
    } else {
        Ok(())
    }
}
pub(super) async fn writable(
    client: &Client,
    grant: &Grant,
    id: &ruma::RoomId,
) -> Result<Room, String> {
    guard(client, grant)?;
    grant.authorize(Capability::Publish)?;
    let room = client
        .get_room(id)
        .ok_or("This chat is no longer joined.")?;
    if room.state() != RoomState::Joined || crate::moments::is_moments(&room) {
        return Err("This chat is no longer joined.".into());
    }
    let member = room
        .get_member(&grant.owner)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Unable to confirm room membership.")?;
    if !member.can_send_message(ruma::events::MessageLikeEventType::RoomMessage) {
        return Err("You cannot publish in this chat.".into());
    }
    guard(client, grant)?;
    if room.state() != RoomState::Joined {
        return Err("This chat is no longer joined.".into());
    }
    Ok(room)
}
fn persist(grant: &Grant, operation: &Operation) -> Result<(), String> {
    storage::update(crate::app_data_dir(), grant, |lib| {
        if let Some(old) = lib.outbox.iter_mut().find(|o| o.id == operation.id) {
            *old = operation.clone()
        } else {
            lib.outbox.retain(|o| !o.finished);
            lib.outbox.push(operation.clone());
        }
        Ok(())
    })
}

/// Fetch all replacements before claiming a complete withdrawal; never silently
/// stop at the first page. Only the original author's valid edits are relevant.
async fn replacements(
    client: &Client,
    grant: &Grant,
    room: &Room,
    root: &ruma::EventId,
    owner: &ruma::UserId,
) -> Result<Vec<(OwnedEventId, Value)>, String> {
    let mut from = None;
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    loop {
        guard(client, grant)?;
        let response = room
            .relations(
                root.to_owned(),
                RelationsOptions {
                    from: from.clone(),
                    include_relations: IncludeRelations::RelationsOfType(RelationType::Replacement),
                    limit: Some(100u32.into()),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        for event in response.chunk {
            let v: Value =
                serde_json::from_str(event.raw().json().get()).map_err(|e| e.to_string())?;
            if v["sender"].as_str() != Some(owner.as_str()) {
                continue;
            }
            if v["content"]["m.relates_to"]["event_id"].as_str() != Some(root.as_str()) {
                continue;
            }
            if let Some(id) = event.event_id() {
                result.push((id.to_owned(), v));
            }
        }
        match response.next_batch_token {
            Some(token) => {
                if !seen.insert(token.clone()) || seen.len() > 100 {
                    return Err("Too many article revisions to verify safely.".into());
                }
                from = Some(token)
            }
            None => break,
        }
    }
    // Some homeservers (including the fixture Palpo) acknowledge m.replace
    // without populating /relations. Scan backwards to the original as well,
    // using SDK-decrypted events so this also works in encrypted rooms. A
    // bounded/incomplete scan is an error, never permission to show stale data
    // or claim a complete withdrawal.
    let mut from = None;
    let mut history_tokens = BTreeSet::new();
    let mut found_root = false;
    for _ in 0..100 {
        guard(client, grant)?;
        let mut options = matrix_sdk::room::MessagesOptions::backward();
        options.from = from;
        options.limit = 100u32.into();
        let page = room.messages(options).await.map_err(|e| e.to_string())?;
        for event in page.chunk {
            let Some(id) = event.event_id() else {
                continue;
            };
            if id == root {
                found_root = true;
                break;
            }
            let value: Value =
                serde_json::from_str(event.raw().json().get()).map_err(|e| e.to_string())?;
            if value["sender"].as_str() == Some(owner.as_str())
                && value["content"]["m.relates_to"]["event_id"].as_str() == Some(root.as_str())
                && value["content"]["m.relates_to"]["rel_type"].as_str() == Some("m.replace")
                && !result.iter().any(|(existing, _)| existing == &id)
            {
                result.push((id.to_owned(), value));
            }
        }
        if found_root {
            break;
        }
        let Some(token) = page.end else {
            break;
        };
        if !history_tokens.insert(token.clone()) {
            break;
        }
        from = Some(token);
    }
    if !found_root {
        return Err(
            "Unable to verify the complete article history. Try again when it is available.".into(),
        );
    }
    Ok(result)
}
pub async fn execute(
    client: Client,
    grant: Grant,
    mut op: Operation,
) -> Result<Publication, String> {
    let _lock = WRITES.lock().await;
    guard(&client, &grant)?;
    grant.authorize(Capability::Publish)?;
    let library = storage::load(crate::app_data_dir(), &grant)?;
    if let Some(previous) = library.outbox.iter().find(|o| o.id == op.id) {
        op = previous.clone();
    }
    if library
        .outbox
        .iter()
        .any(|o| !o.finished && o.id != op.id && o.document.id == op.document.id)
    {
        return Err("Resolve the pending publication before starting another operation.".into());
    }
    if op.finished {
        return library
            .publications
            .iter()
            .find(|p| Some(&p.id) == op.publication.as_ref())
            .cloned()
            .ok_or("Publication record is missing".into());
    }
    persist(&grant, &op)?;
    let room = writable(&client, &grant, &op.room).await?;
    let existing = op
        .publication
        .as_ref()
        .and_then(|id| library.publications.iter().find(|p| &p.id == id))
        .cloned();
    if op.kind != OperationKind::Publish && existing.is_none() {
        return Err("Publication record is missing".into());
    }
    if let Some(p) = &existing {
        if p.withdrawn && op.kind != OperationKind::Withdraw {
            return Err("This article has been withdrawn.".into());
        }
        let original = room
            .event(&p.root, Some(RequestConfig::new().retry_limit(0)))
            .await
            .map_err(|e| e.to_string())?;
        if original.sender().as_ref() != Some(&grant.owner) {
            return Err("Only the article author can change it.".into());
        }
        let raw: Value =
            serde_json::from_str(original.raw().json().get()).map_err(|e| e.to_string())?;
        if op.kind == OperationKind::Update && raw["unsigned"].get("redacted_because").is_some() {
            return Err("This article has been withdrawn.".into());
        }
        let revisions = replacements(&client, &grant, &room, &p.root, &grant.owner).await?;
        if op.kind == OperationKind::Update && op.confirmed.is_none() {
            if let Some((id, _)) = revisions.iter().find(|(_, value)| {
                ArticleContent::parse(&value["content"]).is_ok_and(|article| {
                    article.transaction_id.as_deref() == Some(op.id.as_str())
                        && article.document == op.document
                        && article.version == op.version
                })
            }) {
                op.confirmed = Some(id.clone());
                persist(&grant, &op)?;
            }
        }
        if op.kind == OperationKind::Update
            && revisions.iter().any(|(id, v)| {
                !p.events.contains(id)
                    && op.confirmed.as_ref() != Some(id)
                    && v["unsigned"].get("redacted_because").is_none()
                    && v["unsigned"]["transaction_id"].as_str() != Some(&op.id)
            })
        {
            return Err("This article changed on another device. Reopen its latest version before updating.".into());
        }
        if op.kind == OperationKind::Withdraw {
            let mut ids: BTreeSet<_> = p.events.iter().cloned().collect();
            ids.extend(revisions.into_iter().map(|v| v.0));
            // Hide the original first. Remaining revisions remain a resumable task.
            let ordered = std::iter::once(p.root.clone())
                .chain(ids.into_iter().filter(|id| id != &p.root))
                .collect::<Vec<_>>();
            for id in ordered {
                if op.redacted.contains(&id) {
                    continue;
                }
                guard(&client, &grant)?;
                let transaction = ruma::OwnedTransactionId::from(format!(
                    "{}-{}",
                    op.id,
                    blake3::hash(id.as_bytes()).to_hex()
                ));
                room.redact(
                    &id,
                    Some("Article withdrawn by its author"),
                    Some(transaction),
                )
                .await
                .map_err(|e| e.to_string())?;
                op.redacted.push(id);
                persist(&grant, &op)?;
            }
            let mut publication = p.clone();
            publication.withdrawn = true;
            publication.modified = now();
            op.finished = true;
            storage::update(crate::app_data_dir(), &grant, |lib| {
                *lib.publications
                    .iter_mut()
                    .find(|x| x.id == publication.id)
                    .ok_or("Publication record is missing")? = publication.clone();
                *lib.outbox
                    .iter_mut()
                    .find(|x| x.id == op.id)
                    .ok_or("Publication task is missing")? = op.clone();
                Ok(())
            })?;
            return Ok(publication);
        }
    }
    op.document.ready()?;
    let encrypted = room
        .latest_encryption_state()
        .await
        .map_err(|e| e.to_string())?
        .is_encrypted();
    for id in op.document.asset_ids() {
        if op.uploaded.contains_key(&id) {
            continue;
        }
        let asset = library
            .assets
            .get(&id)
            .cloned()
            .ok_or("An article image is missing. Replace it before publishing.")?;
        let bytes = storage::asset_bytes(crate::app_data_dir(), &grant, &id)?;
        guard(&client, &grant)?;
        let source = if encrypted {
            let file = client
                .upload_encrypted_file(&mut std::io::Cursor::new(bytes))
                .await
                .map_err(|e| e.to_string())?;
            MediaSource::Encrypted(Box::new(file))
        } else {
            let response = client
                .media()
                .upload(
                    &mime::IMAGE_PNG,
                    bytes,
                    Some(RequestConfig::new().retry_limit(0)),
                )
                .await
                .map_err(|e| e.to_string())?;
            MediaSource::Plain(response.content_uri)
        };
        guard(&client, &grant)?;
        op.uploaded.insert(id, RemoteAsset { asset, source });
        persist(&grant, &op)?;
    }
    let room = writable(&client, &grant, &op.room).await?;
    let encryption_now = room
        .latest_encryption_state()
        .await
        .map_err(|e| e.to_string())?
        .is_encrypted();
    if encryption_now
        && op
            .uploaded
            .values()
            .any(|a| matches!(a.source, MediaSource::Plain(_)))
    {
        return Err("Chat encryption changed. Review the publication again.".into());
    }
    let content = wire_content(&op.document, op.version, &op.uploaded, op.root.as_deref(), Some(&op.id), encryption_now)?;
    guard(&client, &grant)?;
    let event = if let Some(id) = op.confirmed.clone() {
        id
    } else {
        let transaction = ruma::OwnedTransactionId::from(op.id.clone());
        let response = room
            .send_raw("m.room.message", content)
            .with_transaction_id(&transaction)
            .with_request_config(RequestConfig::new().retry_limit(0))
            .await
            .map_err(|e| {
                format!(
                    "{} {}",
                    crate::i18n::tr(
                        "Publication is pending confirmation. Retry safely from its saved task."
                    ),
                    e
                )
            })?;
        op.confirmed = Some(response.response.event_id.clone());
        persist(&grant, &op)?;
        response.response.event_id
    };
    let mut publication = existing.unwrap_or_else(|| Publication {
        id: new_id(),
        document_id: op.document.id.clone(),
        room: op.room.clone(),
        room_name: op.room_name.clone(),
        root: event.clone(),
        events: vec![],
        version: 0,
        withdrawn: false,
        modified: now(),
        document: op.document.clone(),
        assets: BTreeMap::new(),
    });
    if !publication.events.contains(&event) {
        publication.events.push(event)
    }
    publication.version = op.version;
    publication.document = op.document.clone();
    publication.assets = op.uploaded.clone();
    publication.modified = now();
    op.finished = true;
    op.publication = Some(publication.id.clone());
    storage::update(crate::app_data_dir(), &grant, |lib| {
        if let Some(p) = lib.publications.iter_mut().find(|p| p.id == publication.id) {
            *p = publication.clone()
        } else {
            lib.publications.push(publication.clone())
        }
        *lib.outbox
            .iter_mut()
            .find(|o| o.id == op.id)
            .ok_or("Publication task is missing")? = op.clone();
        Ok(())
    })?;
    Ok(publication)
}
pub async fn read_article(
    client: Client,
    grant: Grant,
    room_id: OwnedRoomId,
    event_id: OwnedEventId,
) -> Result<ArticleContent, String> {
    guard(&client, &grant)?;
    grant.authorize(Capability::ReadPublished)?;
    let room = client
        .get_room(&room_id)
        .ok_or("This chat is not available.")?;
    let event = room
        .event(&event_id, Some(RequestConfig::new().retry_limit(0)))
        .await
        .map_err(|e| e.to_string())?;
    let raw: Value = serde_json::from_str(event.raw().json().get()).map_err(|e| e.to_string())?;
    if raw["unsigned"].get("redacted_because").is_some() {
        return Err("This article has been withdrawn.".into());
    }
    let author = event.sender().ok_or("Missing article author")?;
    let edits = replacements(&client, &grant, &room, &event_id, &author).await?;
    let latest = edits
        .iter()
        .filter(|(_, v)| v["unsigned"].get("redacted_because").is_none())
        .max_by_key(|(id, v)| (v["origin_server_ts"].as_u64().unwrap_or(0), id.as_str()));
    let content = latest
        .map(|(_, v)| &v["content"])
        .unwrap_or(&raw["content"]);
    let article = ArticleContent::parse(content)?;
    guard(&client, &grant)?;
    Ok(article)
}
/// Stream through the logged-in homeserver with an enforced byte ceiling before
/// allocating/decoding. The package never sees either token or encryption keys.
pub async fn download_image(
    client: Client,
    grant: Grant,
    asset: RemoteAsset,
) -> Result<Vec<u8>, String> {
    use std::io::Read;
    guard(&client, &grant)?;
    grant.authorize(Capability::ReadPublished)?;
    if asset.asset.bytes == 0 || asset.asset.bytes > storage::MAX_FILE {
        return Err("Invalid article image".into());
    }
    let uri = match &asset.source {
        MediaSource::Plain(uri) => uri,
        MediaSource::Encrypted(file) => &file.url,
    };
    let (server, media) = uri.parts().map_err(|_| "Invalid article image")?;
    let supported = client
        .supported_versions()
        .await
        .map_err(|e| e.to_string())?;
    let authenticated = supported
        .versions
        .iter()
        .any(|v| *v >= ruma::api::MatrixVersion::V1_11);
    let mut url = client.homeserver();
    {
        let mut path = url.path_segments_mut().map_err(|_| "Invalid homeserver")?;
        path.pop_if_empty();
        if authenticated {
            path.extend([
                "_matrix",
                "client",
                "v1",
                "media",
                "download",
                server.as_str(),
                media,
            ]);
        } else {
            path.extend(["_matrix", "media", "v3", "download", server.as_str(), media]);
        }
    }
    let mut request = client
        .http_client()
        .get(url)
        .timeout(std::time::Duration::from_secs(45));
    if authenticated {
        request = request.bearer_auth(client.access_token().ok_or("Authorization expired")?);
    }
    guard(&client, &grant)?;
    let mut response = request
        .send()
        .await
        .map_err(|_| "Unable to download article image")?
        .error_for_status()
        .map_err(|_| "Unable to download article image")?;
    let limit = asset.asset.bytes.min(storage::MAX_FILE) as usize;
    if response
        .content_length()
        .is_some_and(|len| len > limit as u64)
    {
        return Err("Article image exceeds its size limit".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Unable to download article image")?
    {
        guard(&client, &grant)?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err("Article image exceeds its size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if let MediaSource::Encrypted(file) = &asset.source {
        let mut cursor = std::io::Cursor::new(bytes);
        let reader =
            matrix_sdk_crypto::AttachmentDecryptor::new(&mut cursor, file.as_ref().clone().into())
                .map_err(|_| "Article image failed its integrity check.")?;
        let mut decrypted = Vec::new();
        reader
            .take(storage::MAX_FILE + 1)
            .read_to_end(&mut decrypted)
            .map_err(|_| "Article image failed its integrity check.")?;
        bytes = decrypted;
    }
    if bytes.len() as u64 != asset.asset.bytes
        || blake3::hash(&bytes).to_hex().as_str() != asset.asset.id
    {
        return Err("Article image failed its integrity check.".into());
    }
    let reader =
        image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Png);
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| "Invalid article image")?;
    if width != asset.asset.width
        || height != asset.asset.height
        || width == 0
        || height == 0
        || width > 2048
        || height > 2048
    {
        return Err("Invalid article image dimensions".into());
    }
    guard(&client, &grant)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn encrypt_and_decrypt(wire: &Value) -> Value {
        // Use real SDK Megolm encryption, with long IDs, but no server/account.
        let room = ruma::RoomId::parse(format!("!{}:example.org", "r".repeat(242))).unwrap();
        let machine = matrix_sdk_crypto::OlmMachine::new(
            ruma::user_id!("@article-test:example.org"),
            ruma::device_id!("ARTICLE_TEST"),
        ).await;
        machine.share_room_key(&room, std::iter::empty(), matrix_sdk_crypto::EncryptionSettings::default())
            .await.unwrap();
        let encrypted = machine.encrypt_room_event_raw(
            &room, "m.room.message", &ruma::serde::Raw::new(wire).unwrap().cast_unchecked(),
        ).await.unwrap();
        let bytes = encrypted.content.json().get().len();
        println!("article payload: {} bytes; encrypted content: {bytes} bytes", content_size(wire).unwrap());
        // The encrypted content must leave room for the server's full event.
        assert!(bytes + 4096 <= 65_536, "encrypted content exceeds Matrix event budget: {bytes}");
        let event = json!({
            "event_id": "$article:example.org", "origin_server_ts": 1,
            "sender": machine.user_id(), "type": "m.room.encrypted", "content": encrypted.content,
        });
        let decrypted = machine.decrypt_room_event(
            &ruma::serde::Raw::new(&event).unwrap().cast_unchecked(), &room,
            &matrix_sdk_crypto::DecryptionSettings {
                sender_device_trust_requirement: matrix_sdk_crypto::TrustRequirement::Untrusted,
            },
        ).await.unwrap();
        let event: Value = serde_json::from_str(decrypted.event.json().get()).unwrap();
        event["content"].clone()
    }

    #[tokio::test]
    async fn editor_markdown_fits_after_matrix_encryption() {
        let source = include_str!("../../lab/article-editor/render-comparison/source.md");
        let doc = Document::from_markdown("Markdown 格式测试", source).unwrap();
        let stored = serde_json::to_vec(&doc).unwrap();
        let transaction = "r".repeat(100);
        for root in [None, Some(ruma::event_id!("$original"))] {
            let version = if root.is_some() { 2 } else { 1 };
            let wire = wire_content(&doc, version, &BTreeMap::new(), root, Some(&transaction), true).unwrap();
            assert!(content_size(&wire).unwrap() <= ENCRYPTED_CONTENT_LIMIT);
            let received = encrypt_and_decrypt(&wire).await;
            let article = ArticleContent::parse(&received).unwrap();
            assert_eq!(article.document, doc);
            assert_eq!(article.document.markdown(), source);
            assert_eq!(article.transaction_id.as_deref(), Some(transaction.as_str()));
            assert_eq!(article.version, version);
            let new = received.get("m.new_content").unwrap_or(&received);
            assert!(new["body"].as_str().unwrap().contains("End"));
            assert!(new["body"].as_str().unwrap().contains("多语言代码高亮"));
            // The compact representation still works in schema 2 readers.
            let legacy: Document = serde_json::from_value(new[ARTICLE_KEY]["document"].clone()).unwrap();
            assert_eq!(legacy, doc);
            if root.is_some() {
                assert!(wire.get(ARTICLE_KEY).is_none());
                assert_eq!(received["m.relates_to"]["event_id"], "$original");
            }
        }
        assert_eq!(serde_json::to_vec(&doc).unwrap(), stored);
    }

    #[tokio::test]
    async fn encryption_budget_leaves_room_for_megolm_and_event_metadata() {
        let mut wire = json!({"msgtype": "m.text", "body": "",
            "m.relates_to": {"rel_type": "m.replace", "event_id": format!("${}", "e".repeat(254))}});
        wire["body"] = json!("x".repeat(ENCRYPTED_CONTENT_LIMIT - content_size(&wire).unwrap()));
        assert_eq!(content_size(&wire).unwrap(), ENCRYPTED_CONTENT_LIMIT);
        assert_eq!(encrypt_and_decrypt(&wire).await, wire);
    }

    #[test]
    fn compact_wire_keeps_non_default_formatting_and_original_source_hash() {
        let mut doc = Document::from_markdown("Rich", "**bold** *italic* [link](https://example.org)").unwrap();
        doc.blocks[0].width = 75;
        doc.blocks[0].caption = "Caption".into();
        doc.blocks[0].alt = "Alt".into();
        let original_hash = doc.imported_source.as_ref().unwrap().blocks_hash.clone();
        let wire = wire_content(&doc, 1, &BTreeMap::new(), None, Some("retry-123"), true).unwrap();
        let document = &wire[ARTICLE_KEY]["document"];
        assert!(document["blocks"][0].get("asset").is_none());
        assert_eq!(document["blocks"][0]["width"], 75);
        let restored = ArticleContent::parse(&wire).unwrap().document;
        assert_eq!(restored, doc);
        assert_eq!(restored.imported_source.unwrap().blocks_hash, original_hash);
        // Publications from older versions containing all defaults still read.
        let mut old = wire.clone();
        old[ARTICLE_KEY]["document"] = serde_json::to_value(&doc).unwrap();
        assert_eq!(ArticleContent::parse(&old).unwrap().document, doc);
        old["m.new_content"] = wire;
        assert_eq!(ArticleContent::parse(&old).unwrap().document, doc);
    }

    #[test]
    fn large_articles_keep_complete_plain_text_without_html_duplication() {
        let source = "中文 👩‍💻 & \"quoted\" \\ tail\n".repeat(400);
        let doc = Document { title: "Large".into(),
            blocks: vec![Block::new(BlockKind::Paragraph, &source)], ..Document::default() };
        let plain = wire_content(&doc, 1, &BTreeMap::new(), None, None, false).unwrap();
        let encrypted = wire_content(&doc, 1, &BTreeMap::new(), None, Some("retry"), true).unwrap();
        assert!(plain.get("formatted_body").is_some());
        assert!(encrypted.get("formatted_body").is_none());
        assert!(encrypted.get("format").is_none());
        assert_eq!(plain["body"], encrypted["body"]);
        assert_eq!(ArticleContent::parse(&encrypted).unwrap().document, doc);
        assert!(content_size(&encrypted).unwrap() <= ENCRYPTED_CONTENT_LIMIT);
    }

    #[test]
    fn oversized_articles_are_rejected_without_truncation() {
        let source = "中\"\\".repeat(9000);
        let doc = Document { title: "Large".into(),
            blocks: vec![Block::new(BlockKind::Paragraph, &source)], ..Document::default() };
        for root in [None, Some(ruma::event_id!("$original"))] {
            for encrypted in [false, true] {
                assert!(wire_content(&doc, 1, &BTreeMap::new(), root, Some("retry"), encrypted)
                    .unwrap_err().contains("too large"));
            }
        }
        assert_eq!(doc.blocks[0].text, source);
    }
    #[test]
    fn updates_reference_original_and_remain_readable() {
        let doc = Document::from_markdown("文章", "**你好**").unwrap();
        let root = ruma::event_id!("$original");
        let wire = wire_content(&doc, 2, &BTreeMap::new(), Some(root), None, true).unwrap();
        assert_eq!(wire["m.relates_to"]["event_id"], root.as_str());
        assert_eq!(wire["msgtype"], "m.text");
        assert!(wire["m.new_content"]["formatted_body"]
            .as_str()
            .unwrap()
            .contains("<strong>你好</strong>"));
        assert_eq!(ArticleContent::parse(&wire).unwrap().version, 2);
    }
    #[test]
    fn operation_marker_is_optional_but_bounded() {
        let doc = Document::from_markdown("A", "hello").unwrap();
        let mut wire = wire_content(&doc, 1, &BTreeMap::new(), None, None, true).unwrap();
        assert!(ArticleContent::parse(&wire)
            .unwrap()
            .transaction_id
            .is_none());
        wire[ARTICLE_KEY]["transaction_id"] = json!("retry-123");
        assert_eq!(
            ArticleContent::parse(&wire)
                .unwrap()
                .transaction_id
                .as_deref(),
            Some("retry-123")
        );
        wire[ARTICLE_KEY]["transaction_id"] = json!("../../invalid");
        assert!(ArticleContent::parse(&wire).is_err());
    }
    #[test]
    fn remote_media_metadata_must_match_referenced_assets() {
        let mut doc = Document::from_markdown("A", "hello").unwrap();
        let id = "a".repeat(64);
        let remote = RemoteAsset {
            asset: storage::Asset {
                id: id.clone(),
                name: "image".into(),
                width: 1,
                height: 1,
                mime: "image/png".into(),
                bytes: 100,
            },
            source: MediaSource::Plain(ruma::owned_mxc_uri!("mxc://example.org/image")),
        };
        let mut assets = BTreeMap::new();
        assets.insert(id.clone(), remote);
        assert!(ArticleContent::parse(&wire_content(&doc, 1, &assets, None, None, true).unwrap()).is_err());
        let mut block = Block::new(BlockKind::Image, "");
        block.asset = Some(id.clone());
        doc.blocks.push(block);
        assert!(ArticleContent::parse(&wire_content(&doc, 1, &assets, None, None, true).unwrap()).is_ok());
        assets.get_mut(&id).unwrap().asset.width = 0;
        assert!(ArticleContent::parse(&wire_content(&doc, 1, &assets, None, None, true).unwrap()).is_err());
    }
    #[test]
    fn hostile_article_and_local_paths_rejected() {
        let mut doc = Document::default();
        doc.title = "Bad".into();
        let mut b = Block::new(BlockKind::Image, "");
        b.asset = Some("../../secret".into());
        doc.blocks.push(b);
        assert!(wire_content(&doc, 1, &BTreeMap::new(), None, None, true).is_err());
    }
}
