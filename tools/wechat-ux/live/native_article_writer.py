#!/usr/bin/env python3
"""Check the current Markdown writer using hidden native Makepad windows.

Copies only a synthetic @robrix_ux_ fixture profile. Never publishes a post.
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
from native_article_markdown import audit_desktop_runtime_logs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("target/release/rinx"))
    parser.add_argument("--mode", choices=["desktop", "mobile", "both"], default="both")
    args = parser.parse_args()
    assert (args.profile / "latest_user_id.txt").read_text().strip().startswith("@robrix_ux_")
    root = Path("target/article-writer-native") / uuid.uuid4().hex
    root.mkdir(parents=True, mode=0o700)
    report = {"passed": False, "checks": [], "mode": "hidden native Metal",
              "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest()}
    source = Path("lab/article-editor/render-comparison/source.md").read_text()
    swift = root / "visible.swift"
    swift.write_text('''import CoreGraphics
import Foundation
let pid = Int32(CommandLine.arguments[1])!
let windows = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]] ?? []
print(windows.filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid }.count)
''')
    helper = root / "visible"
    subprocess.run(["swiftc", str(swift), "-o", str(helper)], check=True, capture_output=True)
    app = None

    def wait_for(check, message):
        for _ in range(160):
            if check():
                return
            time.sleep(.15)
        raise AssertionError(message)

    def shown(widget):
        return [row for row in app.snap() if row["i"] == widget and row.get("v", 1) != 0]

    def fill(widget, text):
        app.click_id(widget)
        app.request("/k", c="A", cmd=1, wait=1)
        app.request("/k", c="Backspace", wait=1)
        for i in range(0, len(text), 1000):
            app.request("/t", t=text[i:i + 1000], wait=1)

    def mark(name):
        report["checks"].append(name)
        print("PASS", name, flush=True)

    def capture(name):
        assert int(subprocess.check_output([str(helper), str(app.process.pid)])) == 0
        app.capture(name)
        (root / (name + ".json")).write_text(json.dumps(app.snap(), ensure_ascii=False, indent=2))

    try:
        for mode in (["desktop", "mobile"] if args.mode == "both" else [args.mode]):
            profile = root / mode / "profile"
            shutil.copytree(args.profile, profile, ignore=shutil.ignore_patterns("*.sqlite3", "*.sqlite3-wal", "*.sqlite3-shm"))
            for path in args.profile.rglob("*.sqlite3"):
                with sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True) as original:
                    with sqlite3.connect(profile / path.relative_to(args.profile)) as copy:
                        original.backup(copy)
            width, height = (1440, 960) if mode == "desktop" else (390, 844)
            (profile / "window_geom_state.json").write_text(json.dumps({"inner_size": [width, height], "position": [50, 50], "is_fullscreen": False}))
            (profile / "ui-language.json").write_text('"en"')
            for path in profile.glob("*/persistent_state/latest_app_state.json"):
                state = json.loads(path.read_text())
                state.setdefault("app_prefs", {})["view_mode"] = "ForceWide" if mode == "desktop" else "ForceNarrow"
                path.write_text(json.dumps(state))
            with socket.socket() as probe:
                probe.bind(("127.0.0.1", 0))
                port = probe.getsockname()[1]
            app = NativeApp(root, port, auto_login=False)
            app.output.mkdir(parents=True)
            app.log = (app.output / "native.log").open("w")
            env = dict(os.environ, RINX_DATA_DIR=str(profile.resolve()), ROBRIX_DATA_DIR=str(profile.resolve()),
                       MAKEPAD_REMOTE=str(port), MAKEPAD_HIDE_WINDOWS="1", MAKEPAD_NO_FOCUS="1")
            env.pop("MAKEPAD_FOCUS", None)
            app.process = subprocess.Popen([str(args.binary.resolve())], env=env, stdin=subprocess.DEVNULL, stdout=app.log, stderr=subprocess.STDOUT)
            for _ in range(160):
                assert app.process.poll() is None, "Instrumented app exited"
                try:
                    if app.request("/s")["pid"] == app.process.pid and shown("discover_tab" if mode == "mobile" else "article_editor_button"):
                        break
                except OSError:
                    pass
                time.sleep(.2)
            else:
                raise AssertionError("Fixture did not restore")
            if mode == "mobile":
                app.click_id("discover_tab")
                app.click_id("discover_article")
            else:
                app.click_id("article_editor_button")
            app.click_id("article_continue")
            app.click_id("article_allow")
            app.click_id("article_new")
            wait_for(lambda: shown("write_source"), "New document did not open the current Markdown writer")
            capture(mode + "-writer")
            controls = ["article_back", "write_publish", "write_mode_source", "write_mode_preview"]
            controls += ["write_style"] if mode == "desktop" else ["wb_bold", "wb_image", "wb_link"]
            for widget in controls:
                rows = shown(widget)
                assert rows, (mode, "missing control", widget)
                row = rows[0]
                x, y, w, h = row["r"]
                assert x >= 0 and x + w <= width + 1 and y >= 0 and y + h <= height, (mode, widget, row["r"])
            mark(mode + "_writer_controls_reachable")
            title_widget = "write_title_small" if mode == "mobile" else "write_title"
            title = "Latest writer " + mode
            fill(title_widget, title)
            fill("write_source", source)
            app.click_id("write_mode_preview")
            wait_for(lambda: any("Editor.md</h1>" in row.get("t", "") for row in app.snap()), "Full sample did not render")
            capture(mode + "-preview")
            app.click_id("write_mode_source")
            assert shown("write_source")[0]["val"] == source
            mark(mode + "_complete_markdown_preview_and_source")
            if mode == "desktop":
                app.click_id("write_mode_split")
                assert shown("write_source") and shown("write_list")
                mark("desktop_split_view")
            if mode == "desktop":
                app.click_id("write_style")
                wait_for(lambda: any(row.get("t") == "Magazine" for row in app.snap()), "Article styles are not reachable")
                app.click_text("Magazine")
                app.click_id("write_themes_close")
            app.click_id("write_publish")
            wait_for(lambda: shown("review_continue"), "Publish did not open the review step")
            capture(mode + "-review")
            mark(mode + "_publication_review")
            library_path = next(profile.glob("mini-apps/**/library-v2.json"))
            library = json.loads(library_path.read_text())
            doc = next(doc for doc in library["documents"] if doc["title"] == title)
            assert doc["imported_source"]["text"] == source
            assert doc["theme"] == ("magazine" if mode == "desktop" else "classic")
            assert not any(op["document"]["id"] == doc["id"] for op in library["outbox"])
            mark(mode + "_saved_full_source_without_publishing")
            app.stop()
            app = None
        report.update(audit_desktop_runtime_logs(root))
        report["passed"] = True
    finally:
        if app:
            try:
                capture("failure")
            finally:
                app.stop()
        (root / "result.json").write_text(json.dumps(report, indent=2))
        print(root / "result.json", flush=True)


if __name__ == "__main__":
    main()
