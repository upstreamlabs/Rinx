//! Refresh a local unsigned OctoSense bundle digest after editing its sources.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("Usage: rinx-miniapp-package BUNDLE_DIR")?,
    );
    let path = dir.join("manifest.json");
    let mut manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
    if !manifest["integrity"]["signature"].is_null() {
        return Err("This tool does not modify signed manifests".into());
    }
    let digest = octosense_app_policy::digest_dir(&dir)?;
    manifest["integrity"]["bundle_blake3"] = digest.into();
    let json = serde_json::to_string_pretty(&manifest)?;
    let parsed = octosense_app_policy::AppManifest::parse(&json)?;
    octosense_app_policy::policy::resolve(
        &parsed,
        &octosense_app_policy::HostLimits {
            require_signature: false,
            ..Default::default()
        },
    )?;
    std::fs::write(&path, json + "\n")?;
    println!("Packaged {} {}", parsed.id, parsed.version);
    Ok(())
}
