#!/usr/bin/env python3
"""Check the native Wayland path in an isolated headless compositor."""
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time

root = Path(__file__).resolve().parents[1]

with tempfile.TemporaryDirectory(prefix="bug-wayland-") as tmp:
    env = os.environ.copy()
    env.pop("DISPLAY", None)
    env.update(XDG_RUNTIME_DIR=tmp, WAYLAND_DISPLAY="bug-test", XDG_SESSION_TYPE="wayland")
    with open(Path(tmp) / "weston.log", "w+") as compositor_log, open(Path(tmp) / "bug.log", "w+") as bug_log:
        compositor = subprocess.Popen([
            "weston", "--backend=headless", "--renderer=pixman", "--socket=bug-test",
            "--idle-time=0", "--width=1024", "--height=768", "--no-config",
        ], env=env, stdout=compositor_log, stderr=subprocess.STDOUT)
        child = None
        try:
            deadline = time.monotonic() + 15
            while not (Path(tmp) / "bug-test").exists():
                assert compositor.poll() is None, "compositor exited"
                assert time.monotonic() < deadline, "compositor startup timed out"
                time.sleep(0.05)
            child = subprocess.Popen([
                str(root / "target/debug/bug"), "--profile", str(Path(tmp) / "profile"),
                (root / "examples/smoke.html").as_uri(),
            ], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=bug_log, text=True)
            events = queue.Queue()
            def read():
                for line in child.stdout:
                    try: events.put(json.loads(line))
                    except json.JSONDecodeError: events.put({"event":"invalid-json", "line":line})
                events.put({"event":"eof"})
            threading.Thread(target=read, daemon=True).start()
            def wait(predicate):
                deadline = time.monotonic() + 45
                while time.monotonic() < deadline:
                    event = events.get(timeout=max(0.01, deadline-time.monotonic()))
                    assert event["event"] not in ("error", "invalid-json", "eof"), event
                    if predicate(event): return event
                raise AssertionError("timed out")
            wait(lambda e: e["event"] == "ready")
            view = wait(lambda e: e["event"] == "opened")["view"]
            wait(lambda e: e["event"] == "loaded" and e["view"] == view)
            child.stdin.write(json.dumps({"cmd":"eval", "view":view, "script":"document.title='Wayland OK'"}) + '\n')
            child.stdin.flush()
            wait(lambda e: e["event"] == "title" and e["title"] == "Wayland OK")
            # Let several compositor frame callbacks and engine redraws complete.
            time.sleep(0.5)
            child.stdin.write('{"cmd":"quit"}\n')
            child.stdin.flush()
            assert child.wait(timeout=15) == 0
            # Surface presentation errors are also reported on the event stream.
            while True:
                event = events.get(timeout=5)
                assert event["event"] not in ("error", "invalid-json"), event
                if event["event"] == "eof": break
            print("PASS: native Wayland window, page load, JS, presentation without reported errors, shutdown")
        except BaseException:
            for log in (compositor_log, bug_log):
                log.flush()
                log.seek(0)
                print(log.read())
            raise
        finally:
            for process in (child, compositor):
                if process is not None and process.poll() is None:
                    process.terminate()
                    try: process.wait(timeout=10)
                    except subprocess.TimeoutExpired: process.kill(); process.wait()
