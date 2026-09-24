#!/usr/bin/env python3
"""Check phone-layout Moments navigation in a hidden native Metal window.

Uses an isolated copy of an existing @robrix_ux_ profile. No posts are published.
This exercises the mobile layout on macOS, not an iOS/Android device.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import sqlite3
import subprocess
import time
import uuid

from native_probe import NativeApp


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("target/release/rinx"))
    args = parser.parse_args()
    assert (args.profile / "latest_user_id.txt").read_text().strip().startswith("@robrix_ux_")
    root = Path("target/moments-navigation") / uuid.uuid4().hex
    root.mkdir(parents=True, mode=0o700)
    profile = root / "profile"
    shutil.copytree(args.profile, profile, ignore=shutil.ignore_patterns(
        "*.sqlite3", "*.sqlite3-wal", "*.sqlite3-shm"))
    for path in args.profile.rglob("*.sqlite3"):
        with sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True) as source:
            with sqlite3.connect(profile / path.relative_to(args.profile)) as target:
                source.backup(target)
    (profile / "window_geom_state.json").write_text(json.dumps({
        "inner_size": [390, 844], "position": [50, 50], "is_fullscreen": False}))
    (profile / "ui-language.json").write_text(json.dumps("en"))
    for path in profile.glob("*/persistent_state/latest_app_state.json"):
        state = json.loads(path.read_text())
        state.setdefault("app_prefs", {})["view_mode"] = "ForceNarrow"
        path.write_text(json.dumps(state))
    swift = root / "visible-windows.swift"
    swift.write_text('''import CoreGraphics
import Foundation
let pid = Int32(CommandLine.arguments[1])!
let windows = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]] ?? []
print(windows.filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid }.count)
''')
    helper = root / "visible-windows"
    subprocess.run(["swiftc", str(swift), "-o", str(helper)], check=True, capture_output=True)
    report = {"passed": False, "checks": [], "captures": [],
              "mode": "phone layout / hidden macOS Metal window",
              "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest()}
    app = None

    def start():
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        native = NativeApp(root, port, auto_login=False)
        native.output.mkdir(parents=True)
        native.log = (native.output / "native.log").open("w")
        os.chmod(native.output / "native.log", 0o600)
        env = dict(os.environ, RINX_DATA_DIR=str(profile.resolve()),
                   ROBRIX_DATA_DIR=str(profile.resolve()), MAKEPAD_REMOTE=str(port),
                   MAKEPAD_HIDE_WINDOWS="1", MAKEPAD_NO_FOCUS="1")
        env.pop("MAKEPAD_FOCUS", None)
        native.process = subprocess.Popen([str(args.binary.resolve())], env=env,
                                          stdin=subprocess.DEVNULL, stdout=native.log,
                                          stderr=subprocess.STDOUT)
        try:
            for _ in range(160):
                assert native.process.poll() is None, "Instrumented Rinx exited"
                try:
                    assert native.request("/s")["pid"] == native.process.pid
                    if any(w["i"] == "discover_tab" for w in native.snap()):
                        return native
                except OSError:
                    pass
                time.sleep(.25)
            raise AssertionError("Mobile fixture did not restore")
        except BaseException:
            native.stop()
            raise

    def shown(widget):
        return [w for w in app.snap() if w["i"] == widget and w.get("v", 1) != 0]

    def wait_until(check):
        for _ in range(40):
            if check():
                return
            time.sleep(.1)
        raise AssertionError("Navigation did not complete after one Back click")

    def title(text):
        wait_until(lambda: any(w.get("t") == text for w in shown("title")))

    def capture(name):
        assert int(subprocess.check_output([str(helper), str(app.process.pid)])) == 0
        app.capture(name)
        (root / (name + ".json")).write_text(json.dumps(app.snap(), indent=2))
        report["captures"].append(name)

    def passed(name):
        report["checks"].append(name)
        print("PASS", name, flush=True)

    def feed():
        app.click_id("discover_tab")
        app.click_id("discover_moments")
        title("Moments")

    def back_to(text):
        app.click_id("back")
        title(text)

    def close():
        app.click_id("back")
        wait_until(lambda: not shown("moments_compose"))

    draft = "Unsent navigation draft 中文 " + uuid.uuid4().hex[:8]
    try:
        app = start()
        feed()
        capture("feed-before-back")
        close()
        capture("discover-after-back")
        passed("feed_back_closes_in_one_click")

        feed()
        app.click_id("moments_compose")
        title("New Moment")
        app.click_id("moments_body")
        app.request("/k", c="A", cmd=1, wait=1)
        app.request("/t", t=draft, wait=1)
        back_to("Moments")
        passed("composer_back_returns_to_feed_in_one_click")
        app.click_id("moments_compose")
        title("New Moment")
        capture("composer-reopened")
        assert any(w.get("val") == draft for w in shown("moments_body"))
        passed("composer_draft_survives_back_and_reopen")

        app.click_id("compose_audience")
        title("Timeline Audience")
        back_to("New Moment")
        back_to("Moments")
        passed("audience_from_composer_returns_to_composer")
        app.click_id("moments_audience")
        title("Timeline Audience")
        back_to("Moments")
        passed("audience_from_feed_returns_to_feed")
        app.click_id("moments_invites")
        title("Timeline Invitations")
        back_to("Moments")
        close()
        passed("invitations_back_then_feed_back")

        app.click_id("me_tab")
        app.click_id("my_posts")
        title("My Posts")
        close()
        capture("me-after-back")
        passed("my_posts_back_returns_to_me")
        app.stop()
        app = start()
        feed()
        app.click_id("moments_compose")
        title("New Moment")
        assert any(w.get("val") == draft for w in shown("moments_body"))
        capture("draft-after-restart")
        passed("composer_draft_survives_navigation_and_restart")
        report["passed"] = True
    finally:
        if app:
            app.stop()
        logs = "\n".join(p.read_text(errors="replace") for p in root.glob("native-runs/*/native.log"))
        if "panicked at" in logs or "Assertion failed:" in logs:
            report["passed"] = False
        (root / "result.json").write_text(json.dumps(report, indent=2))
        print(root / "result.json", flush=True)
    assert report["passed"]


if __name__ == "__main__":
    main()
