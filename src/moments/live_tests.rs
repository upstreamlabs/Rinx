//! Opt-in integration test: only explicitly supplied @robrix_ux_ fixture users.
//! ROBRIX_MOMENTS_FIXTURE and ROBRIX_DATA_DIR must point to isolated test data.
use super::{
    backend::{Service, Pending, Feed},
    model::{self, Index, Entry},
};
use anyhow::{Result, ensure};
use matrix_sdk::{Client, Room, RoomState, config::SyncSettings};
use ruma::{OwnedEventId, TransactionId, serde::Raw};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

async fn client(
    fixture: &Value,
    key: &str,
    device: &str,
) -> Result<(Service, tokio::task::JoinHandle<()>)> {
    let user = &fixture["users"][key];
    let id = user["user_id"].as_str().unwrap();
    ensure!(
        id.starts_with("@robrix_ux_"),
        "Only isolated fixture accounts are allowed"
    );
    let root = PathBuf::from(std::env::var("ROBRIX_DATA_DIR")?)
        .join("sdk-devices")
        .join(device);
    let client = crate::sliding_sync::base_client_builder(&root, "moments-fixture-store")
        .homeserver_url(fixture["url"].as_str().unwrap())
        .with_encryption_settings(matrix_sdk::encryption::EncryptionSettings {
            auto_enable_cross_signing: false,
            auto_enable_backups: false,
            backup_download_strategy: matrix_sdk::encryption::BackupDownloadStrategy::OneShot,
        })
        .build()
        .await?;
    let session = root.join("fixture-session.json");
    if session.exists() {
        client
            .matrix_auth()
            .restore_session(
                serde_json::from_slice(&std::fs::read(&session)?)?,
                Default::default(),
            )
            .await?;
    } else {
        client
            .matrix_auth()
            .login_username(id, user["password"].as_str().unwrap())
            .initial_device_display_name(device)
            .send()
            .await?;
        super::backend::write_private(&session, &client.matrix_auth().session().unwrap())?;
    }
    let cloned = client.clone();
    let sync = tokio::spawn(async move {
        let _ = cloned
            .sync(SyncSettings::default().timeout(Duration::from_secs(2)))
            .await;
    });
    tokio::time::sleep(Duration::from_secs(2)).await;
    Ok((Service::for_test(client), sync))
}
async fn wait_room(service: &Service, id: &ruma::RoomId, state: RoomState) -> Result<Room> {
    for _ in 0..80 {
        if let Some(room) = service.client.get_room(id) {
            if room.state() == state
                && (state != RoomState::Joined || room.encryption_state().is_encrypted())
            {
                return Ok(room);
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    anyhow::bail!("Room did not sync to expected membership")
}
async fn post(
    service: &Service,
    room: &ruma::RoomId,
    text: &str,
    paths: Vec<PathBuf>,
) -> Result<(OwnedEventId, Pending)> {
    let timeline = service.validate(room).await?;
    let pending = Pending {
        transaction: TransactionId::new(),
        room: room.to_owned(),
        audience: timeline.audience,
        event_type: "m.room.message".into(),
        content: model::post_content(text, &[]),
        is_post: true,
        draft_body: Some(text.into()),
        paths,
        assets: vec![],
        confirmed: None,
    };
    Ok((service.send(pending.clone()).await?, pending))
}
async fn decrypted(service: &Service, room: &ruma::RoomId, event: &ruma::EventId) -> Result<Value> {
    let sdk = service
        .client
        .get_room(room)
        .ok_or_else(|| anyhow::anyhow!("Missing synced room"))?;
    for _ in 0..80 {
        if let Ok(e) = sdk.event(event, None).await {
            if !e.kind.is_utd() {
                return Ok(serde_json::from_str(e.kind.raw().json().get())?);
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    anyhow::bail!("Recipient could not decrypt fixture event")
}
async fn wire(client: &Client, room: &ruma::RoomId, event: &ruma::EventId) -> Result<Value> {
    let response = client
        .send(ruma::api::client::room::get_room_event::v3::Request::new(
            room.to_owned(),
            event.to_owned(),
        ))
        .await?;
    Ok(serde_json::from_str(response.event.json().get())?)
}
async fn index(service: &Service, room: &ruma::RoomId) -> Result<Index> {
    let sdk = service.client.get_room(room).unwrap();
    let mut options = matrix_sdk::room::MessagesOptions::backward();
    options.limit = ruma::uint!(100);
    let mut index = Index::default();
    for e in sdk.messages(options).await?.chunk {
        index.insert(room, serde_json::from_str(e.kind.raw().json().get())?);
    }
    Ok(index)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Requires an explicit isolated Matrix fixture and private test data directory"]
async fn palpo_moments_recipient_encryption_and_native_seed() -> Result<()> {
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(std::env::var("ROBRIX_MOMENTS_FIXTURE")?)?)?;
    ensure!(
        std::env::var("ROBRIX_DATA_DIR")?.contains("moments"),
        "An isolated moments test directory is required"
    );
    let mut checks = vec![];
    let mut tasks = vec![];
    let (alice, a) = client(&fixture, "alex", "author").await?;
    tasks.push(a);
    let (bob, b) = client(&fixture, "emma", "viewer-one").await?;
    tasks.push(b);
    let (bob2, b2) = client(&fixture, "emma", "viewer-two").await?;
    tasks.push(b2);
    let (outsider, o) = client(&fixture, "leo", "outsider").await?;
    tasks.push(o);
    let (alice2, a2) = client(&fixture, "alex", "author-second-device").await?;
    tasks.push(a2);
    let run=async {
        alice.discard_pending().await?;bob.discard_pending().await?;
        let room=alice.ensure_timeline().await?;wait_room(&alice,&room,RoomState::Joined).await?;
        let author_room=alice.client.get_room(&room).unwrap();
        // Re-running the fixture starts with no viewers, without touching chats.
        for member in alice.validate(&room).await?.members {
            if member.id!=alice.owner {alice.membership(&room,&member.id,false).await?;}
        }
        let (before,_)=post(&alice,&room,"Before this audience joined",vec![]).await?;
        ensure!(wire(&alice.client,&room,&before).await?["type"]=="m.room.encrypted");
        ensure!(wire(&outsider.client,&room,&before).await.is_err());checks.push("outsider_cannot_fetch_encrypted_post");
        alice.membership(&room,&bob.owner,true).await?;wait_room(&bob,&room,RoomState::Invited).await?;
        let invitation_feed=bob.load(Feed::default(),false).await?;
        ensure!(invitation_feed.timelines.get(&room).is_some_and(|t|t.invited));checks.push("typed_invitation_requires_explicit_join");
        wait_room(&bob2,&room,RoomState::Invited).await?;bob2.invitation(&room,true).await?;
        wait_room(&bob,&room,RoomState::Joined).await?;wait_room(&bob2,&room,RoomState::Joined).await?;
        let joined_feed=bob.load(invitation_feed,false).await?;
        ensure!(joined_feed.timelines.get(&room).is_some_and(|t|!t.invited));
        checks.push("invitation_accepted_on_other_device_refreshes_existing_feed");
        tokio::time::sleep(Duration::from_secs(2)).await;
        let (first,pending)=post(&alice,&room,"A quiet afternoon by the lake · 湖边的午后",vec![]).await?;
        ensure!(alice.send(pending).await?==first);checks.push("confirmed_retry_keeps_transaction_and_does_not_duplicate");
        let mut uncertain=alice.pending()?.unwrap();uncertain.confirmed=None;alice.save_pending(&uncertain)?;
        ensure!(alice.send(uncertain).await?==first);checks.push("uncertain_http_retry_returns_same_server_event");
        let value=decrypted(&bob,&room,&first).await?;decrypted(&bob2,&room,&first).await?;
        ensure!(value["content"][super::ROOM_TYPE]["version"]==1);checks.push("two_recipient_devices_decrypt_actual_post");
        let old_wire=wire(&alice.client,&room,&before).await?;
        let before_on_bob=bob.client.get_room(&room).unwrap().decrypt_event(&Raw::new(&old_wire)?.cast_unchecked(),None).await?;
        ensure!(before_on_bob.kind.is_utd());checks.push("new_viewer_has_no_prejoin_post_key");
        let entry=Entry {room:room.clone(),id:first.clone(),sender:alice.owner.clone(),timestamp:1,content:value["content"].clone(),edited:false};
        let comment=bob.interact(&entry,model::comment_content("Beautiful light! 光线真好",&first),"m.room.message",TransactionId::new()).await?;
        ensure!(decrypted(&alice,&room,&comment).await?["sender"]==bob.owner.as_str());
        bob.like(&entry,TransactionId::new()).await?;bob.like(&entry,TransactionId::new()).await?;
        let idx=index(&alice,&room).await?;ensure!(idx.comments(&entry).len()==1);ensure!(idx.likes(&entry).len()==1);
        let reactions=idx.likes(&entry)[&bob.owner].clone();ensure!((1..=2).contains(&reactions.len()));checks.push("encrypted_comment_and_deduplicated_standard_likes");
        bob.redact(&room,reactions).await?;ensure!(index(&alice,&room).await?.likes(&entry).is_empty());checks.push("unlike_redacts_all_duplicate_reactions");
        // A legitimate viewer may send arbitrary encrypted roots. Rinx must
        // not treat those roots as posts by the timeline's owner.
        let forged=bob.client.get_room(&room).unwrap().send_raw("m.room.message",model::post_content("Forged owner post",&[])).await?.response.event_id;
        decrypted(&alice,&room,&forged).await?;
        ensure!(!index(&alice,&room).await?.posts(&room,&alice.owner).iter().any(|p|p.id==forged));checks.push("viewer_cannot_impersonate_author_in_feed");
        let mut edited=entry.content.clone();edited["body"]=json!("A quiet afternoon by the lake · edited");
        let edit=author_room.send_raw("m.room.message",json!({"msgtype":"m.text","body":"* edited","m.new_content":edited,"m.relates_to":{"rel_type":"m.replace","event_id":first}})).await?.response.event_id;
        decrypted(&bob,&room,&edit).await?;
        ensure!(index(&bob,&room).await?.posts(&room,&alice.owner).iter().any(|p|p.id==first&&p.edited));checks.push("same_author_edit_retains_root_identity");
        let photo=PathBuf::from(std::env::var("ROBRIX_DATA_DIR")?).join("fixture-photo.png");
        if !photo.exists(){std::fs::write(&photo,include_bytes!("../../tools/wechat-ux/fixtures/moments-photo.png"))?;}
        let (album,_)=post(&alice,&room,"Weekend album · 周末相册",vec![photo.clone(),photo.clone()]).await?;
        let media_event=decrypted(&bob,&room,&album).await?;
        let assets:Vec<model::Asset>=serde_json::from_value(media_event["content"][super::ROOM_TYPE]["media"].clone())?;
        ensure!(assets.len()==2);
        for asset in &assets {
            let request=matrix_sdk::media::MediaRequestParameters {source:ruma::events::room::MediaSource::Encrypted(Box::new(asset.file.clone())),format:matrix_sdk::media::MediaFormat::File};
            let bytes=bob.client.media().get_media_content(&request,true).await?;ensure!(bytes==std::fs::read(&photo)?);
        }checks.push("ordered_album_attachments_decrypt_and_match_original_bytes");
        let delayed=photo.with_file_name("delayed-upload.png");let _=std::fs::remove_file(&delayed);
        let timeline=alice.validate(&room).await?;
        let upload=Pending{transaction:TransactionId::new(),room:room.clone(),audience:timeline.audience,event_type:"m.room.message".into(),content:model::post_content("Recovered album upload",&[]),is_post:true,draft_body:Some("Recovered album upload".into()),paths:vec![photo.clone(),delayed.clone()],assets:vec![],confirmed:None};
        ensure!(alice.send(upload).await.is_err());let saved=alice.pending()?.unwrap();ensure!(saved.assets.len()==1);
        let first_upload=saved.assets[0].file.url.clone();std::fs::copy(&photo,&delayed)?;
        let resumed=Service::for_test(alice.client.clone());let retry=resumed.send(resumed.pending()?.unwrap()).await?;
        let recovered=decrypted(&bob,&room,&retry).await?;
        ensure!(recovered["content"][super::ROOM_TYPE]["media"][0]["file"]["url"]==first_upload.as_str());
        checks.push("interrupted_album_reuses_persisted_asset_and_transaction");
        // Preserve the old room object and keys on BOTH viewer devices, then
        // obtain ciphertext using the author and attempt decryption directly.
        let retained=bob.client.get_room(&room).unwrap();let retained2=bob2.client.get_room(&room).unwrap();
        alice.membership(&room,&bob.owner,false).await?;
        let (after,_)=post(&alice,&room,"After viewer removal: private again",vec![]).await?;
        let encrypted=wire(&alice.client,&room,&after).await?;
        let first_wire=wire(&alice.client,&room,&first).await?;
        ensure!(encrypted["content"]["session_id"]!=first_wire["content"]["session_id"]);
        for retained in [&retained,&retained2] {
            let result=retained.decrypt_event(&Raw::new(&encrypted)?.cast_unchecked(),None).await?;
            ensure!(result.kind.is_utd(),"Removed viewer unexpectedly decrypted future post");
        }
        let removed_viewer_can_fetch_ciphertext=wire(&bob.client,&room,&after).await.is_ok();
        checks.push("removed_viewer_with_old_keys_cannot_decrypt_future_post_on_either_device");
        // An old audience fingerprint is rejected and retained for explicit review.
        let stale=Pending{transaction:TransactionId::new(),room:room.clone(),audience:"stale audience".into(),event_type:"m.room.message".into(),content:model::post_content("Reviewed audience draft",&[]),is_post:true,draft_body:Some("Reviewed audience draft".into()),paths:vec![],assets:vec![],confirmed:None};
        ensure!(alice.send(stale.clone()).await.is_err());ensure!(alice.pending()?.unwrap().transaction==stale.transaction);
        ensure!(alice.review_pending(&room,"stale").await.is_err());
        let reviewed=alice.validate(&room).await?;alice.review_pending(&room,&reviewed.audience).await?;alice.send(alice.pending()?.unwrap()).await?;checks.push("audience_change_blocks_saved_draft_until_explicit_review");
        let transfer=alice.file_transfer().await?;ensure!(transfer!=room);let transfer_room=wait_room(&alice,&transfer,RoomState::Joined).await?;
        wait_room(&alice2,&transfer,RoomState::Joined).await?;
        ensure!(alice2.file_transfer().await?==transfer);
        let note=transfer_room.send_raw("m.room.message",json!({"msgtype":"m.text","body":"Private file transfer note"})).await?.response.event_id;
        ensure!(decrypted(&alice2,&transfer,&note).await?["content"]["body"]=="Private file transfer note");
        ensure!(wire(&bob.client,&transfer,&note).await.is_err());checks.push("separate_private_file_transfer_syncs_to_own_second_device_only");
        let encrypted_file=alice.client.upload_encrypted_file(&mut std::fs::File::open(&photo)?).await?;
        let file_event=transfer_room.send_raw("m.room.message",json!({"msgtype":"m.file","body":"private-transfer.png","file":encrypted_file,"info":{"mimetype":"image/png"}})).await?.response.event_id;
        let file_content=decrypted(&alice2,&transfer,&file_event).await?;
        let source=ruma::events::room::MediaSource::Encrypted(Box::new(serde_json::from_value(file_content["content"]["file"].clone())?));
        let bytes=alice2.client.media().get_media_content(&matrix_sdk::media::MediaRequestParameters{source,format:matrix_sdk::media::MediaFormat::File},true).await?;
        ensure!(bytes==std::fs::read(&photo)?);ensure!(wire(&bob.client,&transfer,&file_event).await.is_err());
        ensure!(!index(&alice,&room).await?.contains(&file_event));checks.push("actual_private_file_reaches_second_own_device_and_never_moments");
        // Leave a joined audience and readable posts for subsequent native tests.
        alice.membership(&room,&bob.owner,true).await?;wait_room(&bob,&room,RoomState::Invited).await?;
        bob.invitation(&room,true).await?;wait_room(&bob,&room,RoomState::Joined).await?;tokio::time::sleep(Duration::from_secs(2)).await;
        let (native,_)=post(&alice,&room,"Native feed fixture · 朋友圈",vec![]).await?;
        decrypted(&bob,&room,&native).await?;
        let bob_room=bob.ensure_timeline().await?;wait_room(&bob,&bob_room,RoomState::Joined).await?;
        if !bob.validate(&bob_room).await?.members.iter().any(|m|m.id==alice.owner){bob.membership(&bob_room,&alice.owner,true).await?;}
        super::backend::write_private(&PathBuf::from(std::env::var("ROBRIX_DATA_DIR")?).join("live-result.json"),&json!({"passed":true,"checks":checks,"removed_viewer_can_fetch_ciphertext":removed_viewer_can_fetch_ciphertext,"timeline":room,"friend_timeline":bob_room,"file_transfer":transfer,"native_event":native,"album":album}))?;
        Ok::<(),anyhow::Error>(())
    }.await;
    for task in tasks {
        task.abort();
    }
    run?;
    println!("Moments live integration: {} checks passed", checks.len());
    Ok(())
}
