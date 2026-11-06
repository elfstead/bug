#!/usr/bin/env python3
"""Exercise JSON as a controller of display-free views, including actual frames."""
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time

root = Path(__file__).resolve().parents[1]
binary = Path(os.environ.get("BUG_TEST_BINARY", root / "target/debug/bug"))
env = os.environ.copy()
for name in ("DISPLAY", "WAYLAND_DISPLAY", "XDG_SESSION_TYPE"):
    env.pop(name, None)

with tempfile.TemporaryDirectory(prefix="bug-headless-") as tmp:
    with open(Path(tmp) / "stderr", "w+") as log:
        child = subprocess.Popen([str(binary), "--headless", "--size", "800x600", "--profile", str(Path(tmp)/"profile"),
                                  (root / "examples/smoke.html").as_uri()],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True, env=env)
        events = queue.Queue()
        observed = []
        def read():
            for line in child.stdout:
                try: events.put(json.loads(line))
                except json.JSONDecodeError: events.put({"event":"invalid-json", "line":line})
            events.put({"event":"eof"})
        threading.Thread(target=read, daemon=True).start()
        def send(command):
            child.stdin.write(json.dumps(command) + '\n')
            child.stdin.flush()
        def wait(predicate, allow_error=False):
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                event = events.get(timeout=max(0.01, deadline-time.monotonic()))
                observed.append(event)
                assert event["event"] not in ("invalid-json", "eof"), event
                assert allow_error or event["event"] != "error", event
                if predicate(event): return event
            raise AssertionError("timed out")
        def screenshot(view, width, height):
            path = Path(tmp) / "frame.ppm"
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                send({"cmd":"screenshot", "view":view, "path":str(path)})
                event = wait(lambda e: e["event"] in ("screenshot", "error"), allow_error=True)
                if event["event"] == "error":
                    assert event["message"] == "view has not produced a frame yet", event
                else:
                    data = path.read_bytes().split(b'\n', 3)
                    if event["width"] == width and event["height"] == height:
                        assert data[0] == b'P6' and data[1] == f'{width} {height}'.encode() and data[2] == b'255'
                        assert len(data[3]) == width*height*3
                        colors = {data[3][i:i+3] for i in range(0, len(data[3]), 3)}
                        if b'\xee\xf3\xeb' in colors and b'\x57\x7c\x50' in colors:
                            (root / 'build/headless-smoke.ppm').write_bytes(path.read_bytes())
                            return
                time.sleep(0.03)
            raise AssertionError("no correctly sized rendered frame")
        try:
            wait(lambda e: e["event"] == "ready")
            view = wait(lambda e: e["event"] == "opened")["view"]
            wait(lambda e: e["event"] == "loaded" and e["view"] == view)
            screenshot(view, 800, 600)
            send({"cmd":"resize", "view":view, "width":640, "height":480})
            wait(lambda e: e["event"] == "resized" and e["width"] == 640)
            screenshot(view, 640, 480)
            send({"cmd":"eval", "view":view, "script":"document.title='Headless JS OK'"})
            wait(lambda e: e["event"] == "title" and e["title"] == "Headless JS OK")
            send({"cmd":"set_load_script", "view":view, "script":"document.title='Headless hook OK'"})
            send({"cmd":"reload", "view":view})
            wait(lambda e: e["event"] == "title" and e["title"] == "Headless hook OK")
            send({"cmd":"open", "url":"about:blank"})
            second = wait(lambda e: e["event"] == "opened" and e["view"] != view)["view"]
            send({"cmd":"list"})
            listing = wait(lambda e: e["event"] == "views")
            assert {v["view"] for v in listing["views"]} == {view, second}
            send({"cmd":"focus", "view":second})
            wait(lambda e: e["event"] == "focused" and e["view"] == second)
            send({"cmd":"close", "view":second})
            wait(lambda e: e["event"] == "closed" and e["view"] == second)
            send({"cmd":"close", "view":view})
            wait(lambda e: e["event"] == "closed" and e["view"] == view)
            assert child.wait(timeout=15) == 0
            print("PASS: headless without display, rendered pixels, resize, JS, load hooks, multiple views, focus, close-last-view exit")
        except BaseException:
            log.flush(); log.seek(0)
            print(log.read()); print("Last events:", observed[-15:])
            raise
        finally:
            if child.poll() is None:
                child.terminate()
                try: child.wait(timeout=10)
                except subprocess.TimeoutExpired: child.kill(); child.wait()
