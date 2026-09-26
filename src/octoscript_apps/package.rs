//! Canonical OctoSense bundles; local import is visibly unsigned, never a
//! claim that a package came from a verified catalog.
use octosense_app_policy::{AppManifest, AppPolicy};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub service: String,
    #[serde(default = "object")]
    pub args: Value,
    pub target: String,
}
fn object() -> Value {
    serde_json::json!({})
}
#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bindings {
    #[serde(default)]
    pub on_open: Vec<Call>,
    #[serde(default)]
    pub events: BTreeMap<String, Call>,
}
pub struct Package {
    pub root: PathBuf,
    pub manifest: AppManifest,
    pub policy: AppPolicy,
    pub source: String,
    pub script: bool,
    pub data: Value,
    pub bindings: Bindings,
    original: PathBuf,
    _snapshot: Snapshot,
}
struct Snapshot(PathBuf);
impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl Package {
    #[cfg(test)]
    pub fn load(root: &Path) -> Result<Self, String> {
        Self::load_in(root, &std::env::temp_dir())
    }
    pub fn load_in(root: &Path, snapshots: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(snapshots).map_err(|e| e.to_string())?;
        // Freeze the admitted bytes. Source edits cannot change a running app's
        // kit, images or bindings after the user reviews its grants.
        let original = root.to_owned();
        let path = snapshots.join(format!("rinx-miniapp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).map_err(|e| e.to_string())?;
        // Own cleanup only after creating the directory: a failed admission
        // must never remove another import's snapshot on a name collision.
        let snapshot = Snapshot(path);
        copy_bundle(root, &snapshot.0, &mut 0, &mut 0, 0)?;
        let root = &snapshot.0;
        let manifest = AppManifest::parse(
            &std::fs::read_to_string(root.join("manifest.json")).map_err(|e| e.to_string())?,
        )?;
        if manifest.agent.is_some() {
            return Err("Bundle agent profiles are not supported here; declare explicit octos.* services instead".into());
        }
        octosense_app_policy::admit_digest(
            &manifest,
            &octosense_app_policy::digest_dir(root)?,
            &octosense_app_policy::RefuseAllSignatures,
        )?;
        let policy = octosense_app_policy::policy::resolve(
            &manifest,
            &octosense_app_policy::HostLimits {
                require_signature: false,
                ..Default::default()
            },
        )?;
        let script = root.join(octosense_app_policy::SCRIPT_ENTRY).is_file();
        let entry = if script {
            octosense_app_policy::SCRIPT_ENTRY
        } else {
            "page.card"
        };
        let source =
            std::fs::read_to_string(root.join(entry)).map_err(|e| format!("{entry}: {e}"))?;
        let data = read_json(root, "page.data.json")?.unwrap_or_else(object);
        if !data.is_object() {
            return Err("page.data.json must be an object".into());
        }
        let bindings: Bindings =
            serde_json::from_value(read_json(root, "bindings.json")?.unwrap_or_else(object))
                .map_err(|e| e.to_string())?;
        if script && (!bindings.on_open.is_empty() || !bindings.events.is_empty()) {
            return Err(
                "main.splash apps call host.request directly; bindings.json is for L0 cards".into(),
            );
        }
        if bindings.on_open.len() > 8 || bindings.events.len() > 64 {
            return Err("Too many service bindings".into());
        }
        for call in bindings.on_open.iter().chain(bindings.events.values()) {
            if !manifest.capabilities.contains(&call.service) {
                return Err(format!("{} is not declared in capabilities", call.service));
            }
            if call.target.is_empty()
                || !call
                    .target
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                return Err("Binding target must be a top-level data name".into());
            }
            if !call.args.is_object() {
                return Err("Binding arguments must be an object".into());
            }
        }
        Ok(Self {
            root: root.to_owned(),
            manifest,
            policy,
            source,
            script,
            data,
            bindings,
            original,
            _snapshot: snapshot,
        })
    }
    pub fn unchanged(&self) -> Result<(), String> {
        let current = Self::load_in(
            &self.original,
            self.root.parent().ok_or("Missing bundle cache")?,
        )?;
        if current.manifest.signing_bytes()? != self.manifest.signing_bytes()? {
            return Err("Package changed; review it again".into());
        }
        Ok(())
    }
}
fn read_json(root: &Path, name: &str) -> Result<Option<Value>, String> {
    match std::fs::read_to_string(root.join(name)) {
        Ok(s) => serde_json::from_str(&s)
            .map(Some)
            .map_err(|e| format!("{name}: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
fn copy_bundle(
    source: &Path,
    destination: &Path,
    total: &mut u64,
    count: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth > 32 {
        return Err("Bundle exceeds 32 directory levels".into());
    }
    *count += 1;
    if *count > 4096 {
        return Err("Bundle exceeds 4096 entries".into());
    }
    let metadata = std::fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("Bundle symlinks are not allowed".into());
    }
    if metadata.is_dir() {
        if depth != 0 {
            std::fs::create_dir(destination).map_err(|e| e.to_string())?;
        }
        for entry in std::fs::read_dir(source).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            copy_bundle(
                &entry.path(),
                &destination.join(entry.file_name()),
                total,
                count,
                depth + 1,
            )?;
        }
    } else if metadata.is_file() {
        use std::io::Read;
        // Bound the read itself, even if the source grows while importing.
        let mut bytes = Vec::new();
        std::fs::File::open(source)
            .map_err(|e| e.to_string())?
            .take(32 * 1024 * 1024 + 1 - *total)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        *total += bytes.len() as u64;
        if *total > 32 * 1024 * 1024 {
            return Err("Bundle exceeds 32 MiB".into());
        }
        std::fs::write(destination, bytes).map_err(|e| e.to_string())?;
    } else {
        return Err("Bundle contains a special file".into());
    }
    Ok(())
}
pub fn arguments(
    template: &Value,
    data: &Value,
    state: &octoscript_ui_l0::InstanceStore,
    key: &str,
    payload: &Value,
) -> Result<Value, String> {
    Ok(match template {
        Value::Object(o) if o.len() == 1 && o.contains_key("$data") => data
            .pointer(o["$data"].as_str().ok_or("Invalid data pointer")?)
            .cloned()
            .ok_or("Missing binding data")?,
        Value::Object(o) if o.len() == 1 && o.contains_key("$state") => state
            .get(key, o["$state"].as_str().ok_or("Invalid state field")?)
            .or_else(|| {
                state.get(
                    octoscript_ui_l0::CARD_STATE_KEY,
                    o["$state"].as_str().unwrap(),
                )
            })
            .cloned()
            .ok_or("Missing binding state")?,
        Value::Object(o) if o.len() == 1 && o.get("$value") == Some(&Value::Bool(true)) => {
            payload.clone()
        }
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| Ok((k.clone(), arguments(v, data, state, key, payload)?)))
                .collect::<Result<_, String>>()?,
        ),
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|v| arguments(v, data, state, key, payload))
                .collect::<Result<_, _>>()?,
        ),
        value => value.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_bundle_uses_the_shared_entry_and_frozen_assets() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/miniapps/matrix-octos-script");
        let package = Package::load(&path).unwrap();
        assert!(package.script);
        assert!(package.source.contains("host.request(\"matrix.profile\""));
        assert!(package.bindings.events.is_empty());
        std::fs::write(
            package.root.join("main.splash"),
            "Image{src: http_resource(\"{{assets}}/icon.png\")}",
        )
        .unwrap();
        let rendered = octosense_app_policy::script_source(&package.root, "http://127.0.0.1:1234/")
            .unwrap()
            .unwrap();
        assert_eq!(
            rendered,
            "Image{src: http_resource(\"http://127.0.0.1:1234/icon.png\")}"
        );
        assert!(Package::load(&package.root).is_err());
    }
    #[test]
    fn service_results_realize_as_declared_record_state() {
        let source = r#"state result {shape: record}
view root Surface { TextBody(text: result.data.display_name) }
"#;
        let report = octoscript_ui_l0::realize(
            source,
            &serde_json::json!({"result":{"data":{"display_name":"Alice"}}}),
            Default::default(),
        );
        let root = report.complete_root().unwrap();
        assert!(format!("{root:?}").contains("Alice"));
    }
    #[test]
    fn sample_bundle_uses_the_shared_renderer_and_updates_state() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/miniapps/matrix-octos");
        let package = Package::load(&path).unwrap();
        let mut state = octoscript_ui_l0::InstanceStore::default();
        let prepared = octoscript_makepad::l0::prepare_with_state(
            &package.source,
            &package.data,
            &state,
            &path.join("kit"),
        )
        .unwrap();
        let ui = octoscript_makepad::to_makepad_l0_ui(&prepared.tree);
        assert!(ui.contains("Matrix + Octos"));
        assert!(ui.contains("on_change"));
        assert!(octoscript_ui_l0::dispatch_with_data(
            &package.source,
            &mut state,
            octoscript_ui_l0::CARD_STATE_KEY,
            "typing",
            Some(&Value::String("hello".into())),
            &package.data
        ));
        let call = &package.bindings.events["ask"];
        assert_eq!(
            arguments(
                &call.args,
                &package.data,
                &state,
                octoscript_ui_l0::CARD_STATE_KEY,
                &Value::Null
            )
            .unwrap()["text"],
            "hello"
        );
    }
    #[test]
    fn imported_bytes_are_frozen_and_admission_rejects_tampering() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/miniapps/matrix-octos");
        let imported = Package::load(&source).unwrap();
        // An independently admitted copy owns its bytes even if its source
        // folder is edited after review.
        let frozen = Package::load(&imported.root).unwrap();
        std::fs::write(imported.root.join("page.card"), "changed after review").unwrap();
        assert!(frozen.source.contains("Matrix + Octos"));
        assert!(frozen.unchanged().is_err());
        assert!(Package::load(&imported.root).is_err());
    }

    #[test]
    fn binding_arguments_are_data_not_identity() {
        let template = serde_json::json!({"body":{"$value":true},"room_id":{"$data":"/room"}});
        let data = serde_json::json!({"room":"!allowed:example.org"});
        let args = arguments(
            &template,
            &data,
            &Default::default(),
            "root",
            &Value::String("hello".into()),
        )
        .unwrap();
        assert_eq!(
            args,
            serde_json::json!({"body":"hello","room_id":"!allowed:example.org"})
        );
        assert!(
            arguments(
                &serde_json::json!({"$data":"/missing"}),
                &data,
                &Default::default(),
                "root",
                &Value::Null
            )
            .is_err()
        );
    }
}
