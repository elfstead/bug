#!/usr/bin/env python3
"""Real-engine host test. Run under xvfb-run; no fake renderer is substituted."""
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time
from capture_x11 import capture

root = Path(__file__).resolve().parents[1]

def main():
    with tempfile.TemporaryDirectory(prefix="bug-smoke-") as tmp:
        log = open(Path(tmp) / "stderr.log", "w+")
        env = os.environ.copy()
        env.pop("WAYLAND_DISPLAY", None)
        env["XDG_SESSION_TYPE"] = "x11"
        child = subprocess.Popen([
            os.environ.get("BUG_TEST_BINARY", str(root / "target/debug/bug")), "--profile", str(Path(tmp) / "profile"),
            (root / "examples/smoke.html").as_uri(),
        ], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True, env=env)
        events = queue.Queue()
        observed = []
        def read():
            for line in child.stdout:
                try:
                    events.put(json.loads(line))
                except json.JSONDecodeError:
                    events.put({"event":"invalid-json", "line":line})
            events.put({"event":"eof"})
        threading.Thread(target=read, daemon=True).start()

        def wait(predicate, timeout=45):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                event = events.get(timeout=max(0.01, deadline-time.monotonic()))
                observed.append(event)
                assert event["event"] not in ("error", "invalid-json", "eof"), event
                if predicate(event):
                    return event
            raise AssertionError("timed out")

        def send(command):
            child.stdin.write(json.dumps(command) + '\n')
            child.stdin.flush()

        try:
            wait(lambda e: e["event"] == "ready")
            view = wait(lambda e: e["event"] == "opened")["view"]
            wait(lambda e: e["event"] == "loaded" and e["view"] == view)
            # A real native window, with no browser bars, is present on the X server.
            windows = subprocess.check_output(["xdotool", "search", "--name", "Bug renderer smoke test"], env=env, text=True).split()
            assert windows, "no rendered-page window"

            send({"cmd":"eval", "view":view, "script":"document.title = 'Bug FFI OK'"})
            wait(lambda e: e["event"] == "title" and e["title"] == "Bug FFI OK")

            def click(selector):
                send({"cmd":"eval", "view":view, "script":f"var r=document.querySelector({json.dumps(selector)}).getBoundingClientRect();document.title='Bug point:'+Math.round(r.x+r.width/2)+','+Math.round(r.y+r.height/2)"})
                title = wait(lambda e: e["event"] == "title" and e["title"].startswith("Bug point:"))["title"]
                x,y = title.removeprefix("Bug point:").split(',')
                subprocess.run(["xdotool", "mousemove", "--window", windows[0], x, y, "click", "1"], check=True, env=env)

            click('button')
            wait(lambda e: e["event"] == "title" and e["title"] == "Bug click OK")

            # Input travels from the native window through Rust and the C ABI.
            send({"cmd":"eval", "view":view, "script":"document.querySelector('input').focus()"})
            subprocess.run(["xdotool", "windowfocus", "--sync", windows[0]], check=True, env=env)
            time.sleep(0.2)
            subprocess.run(["xdotool", "type", "--clearmodifiers", "--delay", "70", "hello"], check=True, env=env)
            wait(lambda e: e["event"] == "title" and e["title"] == "Bug input: hello")

            def key(chord):
                subprocess.run(["xdotool", "key", "--clearmodifiers", chord], check=True, env=env)

            def palette():
                key('ctrl+shift+p')
                wait(lambda e: e["event"] == "hook" and e.get("kind") == "palette" and e["data"] == "opened")

            def type_text(text):
                subprocess.run(["xdotool", "type", "--clearmodifiers", "--delay", "20", text], check=True, env=env)

            send({"cmd":"eval", "view":view, "script":"document.querySelector('input').select();document.title='clipboard ready'"})
            wait(lambda e: e['event']=='title' and e['title']=='clipboard ready')
            key('ctrl+c')
            assert wait(lambda e:e['event']=='clipboard')['text']=='hello'
            assert subprocess.check_output(['xclip','-selection','clipboard','-o'], env=env, text=True)=='hello'
            key('ctrl+x')
            wait(lambda e:e['event']=='clipboard')
            # External ownership: the test clipboard provider is another process.
            subprocess.run(['xclip','-selection','clipboard'], input='external 日本語 🐛', env=env, text=True, check=True)
            key('ctrl+v')
            wait(lambda e:e['event']=='title' and e['title']=='Bug input: external 日本語 🐛')
            send({"cmd":"eval", "view":view, "script":"document.querySelector('input').value='hello';document.title='clipboard tested'"})
            wait(lambda e:e['event']=='title' and e['title']=='clipboard tested')

            # Host palette text must never become page input, including when
            # dismissing it while a page form field retains DOM focus.
            palette()
            time.sleep(0.15)  # Let the host present the overlay after its input event.
            colors = capture(windows[0], root / 'build/palette.png')
            assert {0x18212b, 0x88cfb0, 0xf3f6f8} <= colors, 'palette not visibly presented'
            type_text('reload')
            key('Escape')
            wait(lambda e: e["event"] == "hook" and e.get("kind") == "palette" and e["data"] == "closed")
            send({"cmd":"eval", "view":view, "script":"document.title='Palette preserved: '+document.querySelector('input').value"})
            wait(lambda e: e["event"] == "title" and e["title"] == "Palette preserved: hello")

            send({"cmd":"set_load_script", "view":view, "script":"document.title='Bug load hook OK'"})
            # The palette executes the public Rust API directly, without a JSON
            # reload command or a script injected into the page.
            palette()
            subprocess.run(['xclip','-selection','clipboard'], input='reload', env=env, text=True, check=True)
            key('ctrl+v')
            key('Return')
            wait(lambda e: e["event"] == "title" and e["title"] == "Bug load hook OK")

            palette()
            type_text('new window')
            key('Return')
            type_text('about:blank')
            key('Return')
            from_palette = wait(lambda e: e["event"] == "opened" and e["view"] != view)["view"]
            send({"cmd":"close", "view":from_palette})
            wait(lambda e: e["event"] == "closed" and e["view"] == from_palette)

            click('a')
            popup = wait(lambda e: e["event"] == "opened" and e["view"] != view)["view"]
            send({"cmd":"close", "view":popup})
            wait(lambda e: e["event"] == "closed" and e["view"] == popup)
            send({"cmd":"open", "url":"about:blank"})
            second = wait(lambda e: e["event"] == "opened" and e["view"] != view)["view"]
            send({"cmd":"list"})
            listing = wait(lambda e: e["event"] == "views")
            assert {p["view"] for p in listing["views"]} == {view, second}
            send({"cmd":"close", "view":second})
            wait(lambda e: e["event"] == "closed" and e["view"] == second)
            send({"cmd":"quit"})
            assert child.wait(timeout=15) == 0
            print("PASS: real Ladybird load, window, JS, pointer, keyboard, external Unicode clipboard/cut/copy/paste, palette paste/input isolation/reload/new window, load hook, link-created window, multiple views, close, shutdown")
        except BaseException:
            if child.poll() is None:
                child.terminate()
                try: child.wait(timeout=10)
                except subprocess.TimeoutExpired: child.kill(); child.wait()
            log.seek(0)
            print(log.read())
            print("Last events:", observed[-15:])
            raise
        finally:
            child.stdin.close()
            child.stdout.close()
            log.close()

if __name__ == "__main__":
    main()
