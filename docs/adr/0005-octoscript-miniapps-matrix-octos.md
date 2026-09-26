# ADR 0005: Octoscript mini apps with Matrix and Octos services

- Date: 2026-09-26
- Status: Accepted; implemented and natively verified, with live model generation blocked by provider authentication
- Supersedes the general-runtime deferral in ADR 0002. The article editor and its grants remain supported.

## Outcome

An OctoSense Octoscript mini-app package can open inside Rinx, retain its native UI and state, call the A2App `matrix.*` API contract against the recipient's Matrix account, and use Octos sessions and tools. The same Rinx implementation serves standalone desktop/mobile builds and Rinx embedded in OctoSense.

## Decision

Use the existing Octoscript checker, kit lowering and Makepad Splash renderer. Reuse A2App's Matrix wire contract and SDK implementations, with Rinx owning authorization and dispatch. A mini app is a distinct instance with its own account, optional room, generation, storage and grants; the outer native Rinx module is not the mini-app identity.

Expose Matrix and Octos through a single host-service boundary. Preserve A2App service names, argument validation and response shapes. Declarative `bindings.json` actions and imperative `host.request` calls converge on this boundary. Existing arbitrary `source sys.*` adapters are not automatically mapped to Matrix or Octos services. Do not rewrite an Octoscript package as a special Rinx UI or infer executable source from a chat message.

Octos is a provider supplied by the host. In OctoSense, reuse the running core connection; a Rinx module must not start another kernel implicitly. Standalone Rinx can explicitly configure the same protocol provider. Lack of a provider is a visible unavailable result, never a fabricated response. A provider handle is distinct from the mini app's session, which belongs to its instance.

The host authenticates request identity. Matrix credentials remain in Rinx and model credentials remain in Octos. A call must satisfy the package declaration, the host's capability limits and the current user's grants. The Matrix API adapter preserves room restrictions, account-change revocation and checks after asynchronous completion. App-to-Octos transfer of room data requires both the relevant Matrix read grant and the Octos grant. Publication remains an explicit host-authorized action.

A host drains only requests belonging to its mini-app instances. Nested Rinx and OctoSense card hosts must not steal each other's callbacks. Closing an instance revokes its leases, cancels pending work and drops late replies; returning from the app does not terminate Rinx or the OctoSense shell.

## Ownership

| Component | Owner |
| --- | --- |
| Octoscript grammar, state transitions and widget kits | Octoscript / Octoscript-Makepad |
| Splash execution, widget rendering and isolate identity | Makepad |
| Portable package, grants and service contracts | Reusable mini-app host library |
| Matrix SDK session and A2App-compatible Matrix implementation | Rinx |
| Octos connection and kernel lifetime | Hosting application / configured standalone provider |
| App presentation, room attachment, sharing and navigation | Rinx |

AI generation is optional. Importing and running an existing app must not require an agent. This implementation imports local Octoscript folders with explicit review. Signed App Hub catalog installation, A2App `.splashapp` import, and A2App watch/share lifecycle compatibility remain separate work; adopting the Matrix contract does not imply those import or lifecycle features exist.

## Verification

1. Check the existing A2App Matrix parser contract, grant denial, cross-room denial, instance revocation and late-reply handling.
2. Run the same Octoscript package through the shared renderer and Rinx host; exercise a state-changing input and a host-service result.
3. Exercise real Matrix and Octos protocol adapters, keeping protocol fixtures distinct from live-service results.
4. Verify standalone and OctoSense-hosted builds and native runtime behavior on macOS and Android. Confirm Back, keyboard handling, close/reopen, and that hosted Rinx reuses the Octos provider.
5. Record unverified platform/service combinations explicitly. A parser test, mocked provider or static screenshot alone is not end-to-end completion.

## Implementation

- `crates/miniapp-core` owns the A2App Matrix request parser, response bounds, instance leases, room grants and account-generation revocation. Matrix SDK adapters live in `src/octoscript_apps/matrix`.
- The native **Mini apps** page imports a digest-verified, bounded snapshot of a local bundle, reviews its services and optional room, and creates one account-scoped Splash instance. Closing it revokes requests and discards late responses.
- Current `main.splash` bundles use App Hub's shared script entry and asset-origin resolver. They call the same host services directly and keep their own script state.
- Octoscript-Makepad prepares the same L0 source, kit and state used by OctoSense cards. Input events update state; declarative bindings populate data fields from host replies. Rinx does not translate the example into a custom native screen.
- AppCard publishes its active `octos-app-transport::shared::Connection`. Hosted Rinx borrows that connection and allocates a distinct Octos session and UUID turns. AppCard replacement/disconnection invalidates the captured handle. Standalone Rinx can explicitly configure a remote provider.
- Only the native host supplies account, room, profile, workspace, session and approval identity. For a local core, account/app storage is allocated beneath the core data root from its native launch configuration (`--data-dir`, `OCTOS_HOME`, or child `HOME/.octos`). Remote connections do not imply shared local storage. The core workspace and explicit read allowlist both name the mini app's canonical data directory; tool network access is disabled. An empty read allowlist would inherit the core profile's broader access and must not be used. Bundle `agent` profiles are rejected until their additional budgets and tool limits can be honored. The mini app cannot call raw kernel RPC or approve its own tools. Runtime core tool permissions still require native approval.
- Makepad provides scoped host-queue draining and source reconciliation into existing named widgets. Android Back is dispatched into native navigation rather than finishing the activity.

## Coordinated source baselines

These changes span cooperating repositories. Rinx pins published dependency commits; OctoSense shells use their standard prepared source layout and source locks. Keep one Makepad package identity and one AppCard transport registry across all modules.

| Repository | Baseline revision |
| --- | --- |
| Rinx | `47f46296` |
| OctoSense desktop | `57f4665c3d025e10465a035fd5221fee2ad143f5` |
| OctoSense ROM Home | `53329db1410d5aea446e5727f575f86c43660c8e` |
| Makepad (OctoSense's locked revision) | `1d3d383e84a66dbb18a4a860f505430c9d5b20f4` |
| Octoscript | `68f6a9df55692b5d8ef8873a12721e279a3f40d6` |
| Octoscript-Makepad | `c4c9682219d5bb549856e35086adf1b354844dc3` |
| OctoSense-System-Apps (AppCard workspace) | `4d99cb589d00207a9a63e32d46de6685843929d1` |
| Octos core | `18fcd3f16e527d2b601d7ae244f056fb711bb6b8` (`2.0.3-rc.12`) |
| App Hub policy | `4605128d46fb982828d8198e0d71d62a39c7d6d6` |

Apply the shells' locked `makepad-contained-apps.patch` (source commit `d7a834c74783e2032bc9e2cb9e693eaa2927b1d9`, SHA-256 `70419d98644af0820816f8bbc480365fa214c71f96eea0dba4b541c7ec9d59a2`) over the Makepad baseline, then the Rinx source-reconciliation and Android navigation changes. Scoped host dispatch now comes from that upstream runtime patch.

The developer checkouts are `Rinx`, `OctoSense-rinx-latest`, `OctoSense-ROM-rinx-latest`, `makepad-rinx-latest`, `octoscript-rinx-latest`, `octoscript-makepad-rinx-latest`, `OctoSense-System-Apps-rinx-latest` and `OctoSense-App-Hub-rinx-latest` under a common parent. The `*-rinx-latest` suffix is a local worktree name; revisions above identify the source baseline, with the coordinated working changes applied.

See [the bundle guide](../../examples/miniapps/README.md) for the app format, service binding contract and example. Desktop builds belong to the OctoSense workspace; Android builds belong to the ROM's `home` workspace. The phone validation package is `dev.makepad.octosense.rinx`, separate from the installed launcher. Installing that APK updates Home application code for this test, not the device's underlying ROM image.

## Validation record (2026-09-26)

Remote `main` was checked again at 10:37 UTC: desktop `57f4665c` and ROM Home `53329db1`. Cargo metadata confirms both validation workspaces resolve the working `Rinx` checkout rather than the shell's released Rinx git pin. Runtime checks use native Metal on the MacBook and GLES on the OnePlus 6, with only the owned app's capture/input endpoints.

| Check | Result |
| --- | --- |
| Portable request parsing, grants, account/room isolation and revocation | 30 tests pass |
| Rinx bundle admission, state updates, stream isolation and turn IDs | 7 tests pass |
| Shared transport, correlated replies and local storage configuration | 33 tests pass |
| AppCard store/render and remaining workspace tests | 45 tests pass; one transport doc test ignored |
| AppCard main library suite | 191 pass, 7 fail, 3 ignored; identical failures reproduced from clean `4d99cb5` app sources against the locked framework sources |
| AppCard workspace check and clippy (`--all-targets --no-deps -- -D warnings`) | Pass |
| Desktop native mini apps | Both bundle formats render; live Matrix profile and Octos session open/history work; continuous Chinese/emoji input, close/reopen, and one shared core process verified |
| Android native mini apps | Both bundle formats render; live Matrix profile, Octos session open/history, continuous input, keyboard dismissal and close/reopen work. A Back key and an actual system edge swipe each return to Rinx Discover without exiting Rinx; one shared core process verified |
| Live model generation | Not passed: the configured DeepSeek provider returns HTTP 401. No generated answer or tool-approval success is claimed |

The seven baseline AppCard failures concern L0 exemplar/capability/theme drift: `every_call_the_lowering_emits_has_a_helper`, `the_nav_exemplar_is_the_card_the_profile_tests_check`, `l0_theme_axes_are_all_answered`, `a_live_source_without_its_capability_is_visibly_wrong`, `a_watch_row_reveals_its_own_remove`, `nav_is_generated_from_an_l0_spec`, and `the_language_reference_lists_every_admitted_theme`. They are not suppressed or weakened by this change.

Native verification caught and fixed lost generated-label updates, missing font fallback in L0 inputs, the core profile's rejection of a workspace outside its read allowlist, and two Android Back deliveries from one key press. Parallel import tests caught a timestamp collision: admitted snapshots now use UUID names, and failed directory creation cannot acquire cleanup ownership over an existing snapshot.

The phone's installed base image is `octosense-202609240442`, matching the latest local OTA in `rom-builds/20260924-display` (Android 15 / SDK 35). This work installs current ROM Home application code as `dev.makepad.octosense.rinx`; it does not flash a new system image or replace the normal `dev.makepad.octosense` launcher. Standalone Rinx is build-checked; its remote-provider UI has not been exercised against a separate live server. Write-side Matrix calls and native tool approvals were not exercised against live services.

### Final validation artifacts

Both release builds passed and were exercised natively. Paths below are relative to the common developer checkout parent; hashes identify the tested artifacts, including the coordinated working changes.

| Artifact | Path | SHA-256 |
| --- | --- | --- |
| macOS executable | `OctoSense/target/release/octosense` | `d3321ce95393c5b606d67b56af932f15aa63e444cef5bdcaee399d7fa77440d5` |
| Android test APK | `OctoSense-ROM-rinx-latest/home/target/android/makepad-android-apk/octosense/apk/octo_senserinx.apk` | `9ebd5487876252695d2a365b70d7c02207617d2cc64979cd8b34386cd19585a8` |

The owned desktop and phone test instances were closed after validation. The Android test APK remains installed for manual testing. The coordinated commits and PRs below publish the tested implementation. Dependency source identities and lockfiles were normalized after native validation; the recorded binary hashes identify the preceding native test builds.

### Published dependency commits

| Component | Commit | Pull request |
| --- | --- | --- |
| Makepad | `3fda5b6f9b48f390bd5396938a95d30ad035cd6d` | [PR](https://github.com/OctoSense-org/makepad/pull/32) |
| Octoscript-Makepad | `b9034566b0607c5bb8c04134d17139f99989e1d0` | [PR](https://github.com/OctoSense-org/OctoScript-Makepad/pull/40) |
| System Apps / AppCard | `1576bec6a585758e48c3252030cad2a9bf9c80db` | [PR](https://github.com/OctoSense-org/OctoSense-System-Apps/pull/4) |
| App Hub | `b0591e2caa0dc5c62927f7b85dbf826a50618a03` | [PR](https://github.com/OctoSense-org/OctoSense-App-Hub/pull/8) |

The selected Makepad commit includes the containment patch already merged in Makepad #30. Consumers therefore clear the old overlay lock instead of applying that patch twice. Rinx Cargo metadata resolves one Makepad runtime, one Octoscript runtime, one AppCard transport, and one Octos core, with no task-specific local paths.
