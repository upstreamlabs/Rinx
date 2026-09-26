# Octoscript mini apps in Rinx

The `matrix-octos-script` example uses the current OctoSense `main.splash` bundle format and calls `host.request` directly. It reads a Matrix profile, opens and hydrates its own session on the shared core, and sends a model turn.

The `matrix-octos` example is an ordinary L0 bundle: `page.card`, `page.data.json`, a kit, the OctoSense `manifest.json`, and declarative `bindings.json`. Rinx uses the shared Octoscript checker/lowerer for L0 and the shared App Hub entry resolver for `main.splash`, with native Makepad Splash widgets for both. Service results are live; the example does not substitute fixtures for Matrix or Octos.

## Try it

1. Log in to Matrix in Rinx. On desktop, choose **Mini apps** in the sidebar. On mobile, choose **Discover → Mini apps**.
2. In OctoSense, open AppCard once to connect its Octos core. Rinx captures that same connection when a mini app starts. Standalone Rinx offers an explicit Octos URL/profile/token form.
3. Enter the full path of the `matrix-octos-script` or `matrix-octos` folder, choose **Review bundle**, and review its declared services. Leave the room empty for this example.
4. Choose **Run**, then **Read my Matrix profile**. Edit the prompt and choose **Ask Octos**. Back revokes the app session and interrupts its active turn.

For `main.splash`, service callbacks receive `{is_ok, data, error}`. The source and its `{{assets}}` URLs follow the same entry contract as App Hub. Script apps own their state and callbacks; `bindings.json` is only for L0.

The importer currently accepts local unsigned bundles after explicit review. It does not install signed App Hub catalog packages or A2App `.splashapp` files. Room grants apply only to the room entered during review. Missing providers and failed calls are shown as errors. OS-specific services such as Mail account management are not exposed by this Rinx adapter. Bundle `agent` profiles are also rejected until all of their tool, budget and permission limits can be enforced. Octos service sessions explicitly narrow filesystem access to this app's data directory and disable tool network access. A remote standalone provider must be able to resolve that directory on its host; otherwise the core refuses the session. Bundled `os.*` system apps with an empty digest do not gain implicit trust when imported; a locally reviewed copy needs its digest generated.

## Bind an event to a service

```json
{
  "events": {
    "refresh": {"service": "matrix.profile", "args": {}, "target": "profile"},
    "ask": {"service": "octos.turn.start", "args": {"text": {"$state": "prompt"}}, "target": "answer"}
  }
}
```

Declare each service in `manifest.json` capabilities. Responses appear in the corresponding data field as `{"is_ok":true,"data":...}` or `{"is_ok":false,"error":...}`. Arguments can reference `{"$state":"field"}`, `{"$data":"/json/pointer"}` or `{"$value":true}`. A binding cannot provide account credentials, a core session ID, or an approval decision. Octos tool approvals use native host controls.

After editing bundle bytes, regenerate the digest:

```sh
cargo build --offline --manifest-path tools/miniapp-package/Cargo.toml
tools/miniapp-package/target/debug/rinx-miniapp-package examples/miniapps/matrix-octos
```

Each admitted instance runs from a verified snapshot. Editing the original package requires reviewing it again. Storage is confined to the current Matrix account and app ID. With a local shared core it lives beneath that core's configured data root, so session access can narrow the profile allowlist without widening it. Matrix-only / remote configurations use Rinx's data root. A custom core profile can still refuse the directory; Rinx does not relax that profile.

The kit files are from Octoscript revision `68f6a9df55692b5d8ef8873a12721e279a3f40d6`; their MIT license is included in `kit/LICENSE`. The Matrix contract and adapters derive from A2App revision `d4d39612fdee574a0f6a33480a19868f1ec85644`, under the license retained in `crates/miniapp-core/LICENSE-MIT`.
