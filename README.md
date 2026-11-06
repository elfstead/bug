# Bug

A programmable, page-only host for Ladybird. **kytin** is an alternate project name.

One page per ordinary window, or an offscreen view with no display server. Your
tiling window manager owns layout and switching; your Rust code or scripts own
navigation and interaction. No tabs, address bar, toolbar, or Qt. Ctrl+Shift+P
opens a transient command palette in the windowed client.

## Architecture

```text
Rust applications / command palette / JSON scripting adapter
           ↕ public Rust methods and typed events
Bug library: Browser, views, scripts, input, CPU frames, profile lifetime
           ↕ ordinary in-process C ABI calls
C++ adapter: LibWebView application and views
           ↕ Ladybird's existing IPC
Ladybird renderer, compositor, and helper processes
```

`winit` + `softbuffer` and the headless loop are replaceable clients of this
library. The library API has no window handles or transport framing. A custom client
can pump it from its own event loop and display frames however it chooses.

The C++ adapter is loaded into the Rust process. There is no added socket or broker
between the host and the engine. The initial presentation path uses CPU-backed
frames; native GPU presentation is a future optimization.

The adapter targets the exact upstream commit in `ladybird-revision`. Ladybird's
frontend interfaces can change, so updates are explicit. Its standard browser UI
is neither constructed nor needed by Bug.

## Build and run

Linux supports native Wayland and X11 windows. Release CI also targets Apple
Silicon macOS. For local development on NixOS:

```sh
cd ~/dev/bug
nix-shell
scripts/build-engine       # first build compiles Ladybird and fetches pinned sources
cargo build --locked
target/debug/bug https://ladybird.org
```

`BUG_JOBS=8 scripts/build-engine` changes the native build parallelism (default 4).
The first engine build is substantial. Subsequent builds are incremental. The
helper binaries and resources remain in `build/engine`; Bug loads
`build/engine/lib/libbug_engine.so`. Use `--engine PATH` to select another build
with ABI version 2. Rebuild the engine after updating an older checkout.
`--help` works without building the engine.

The Nix shell uses your `<nixpkgs>`; Ladybird and Cargo dependencies are pinned,
but the Nix package collection currently follows your configured channel. The
shell reuses the Ladybird package's engine dependencies and explicitly removes
its Qt inputs. `patches/frontend.patch` selects Bug instead of `UI/Qt`, uses
system FFmpeg names outside vcpkg, and permits newer Rust compiler warnings. Preparation refuses
to overwrite an unexpected Ladybird revision or incompatible local edits.
`patches/api.patch` makes Ladybird's internal script-completion method virtual,
allowing the adapter to receive execution results through existing IPC. It does
not start a WebDriver server or add a socket.

Profiles default to `$XDG_DATA_HOME/bug/profile` (or
`~/.local/share/bug/profile`). Use `--profile /absolute/path` for an isolated one.
Only one Bug process can use a profile at a time; a process can own many windows.

## Release builds

[Release CI](.github/workflows/release.yml) builds these Qt-free archives:

| Archive | Target |
| --- | --- |
| `bug-snapshot-<commit>-linux-x64.tar.gz` | Linux x86-64-v2, glibc 2.39+ (Ubuntu 24.04 baseline) |
| `bug-snapshot-<commit>-macos-arm64.tar.gz` | Apple Silicon, macOS 15+ |

Pushes to `master` build and publish a GitHub **prerelease** named
`snapshot-<12-character commit>`, with both platform archives and `SHA256SUMS`.
Both builds must pass before publication. Snapshots are not marked as the latest
stable release. Manual branch runs produce Actions artifacts without publication.
Published releases are never overwritten; failed draft uploads can be retried.

Bug has no numbered milestone release yet. Cargo uses `0.0.0-dev` as an internal
placeholder and registry publication is disabled. `bug --version` reports the
snapshot identity and full commit without loading the engine; local modified
checkouts add `-dirty`. `build.json` in each archive also records the pinned
Ladybird revision. Rust users can read `bug::VERSION` and `bug::COMMIT`.

When a milestone is ready, explicitly push a tag such as `v0.1.0-alpha.1`.
The same workflow uses that tag as the archive/release identity and marks alpha
tags as prereleases. A plain `v0.1.0` would deliberately publish a stable milestone.
Manual runs on milestone tags also publish. ABI and session-format numbers are
independent compatibility markers, not product release numbers.

Extract the entire archive and run `bug-<identity>-<platform>/bin/bug`, optionally with
`--headless`. Keep `bin`, `lib`, resources, and licenses together; the executable
finds its engine beside the installation, without `--engine` or a source checkout.
The archive includes Ladybird's helpers and shared libraries. Linux still uses
the system's glibc, display server, fonts, certificate store, and graphics drivers.
On NixOS, use the development shell above; the Linux release is built for
conventional glibc distributions. macOS archives are ad-hoc signed and are not
notarized with an Apple Developer ID.

This is one download containing several runtime files. Most third-party
libraries are bundled; they do not need to be installed separately. Static
linking could reduce the library files, but helper executables, resources, and
OS interfaces remain. Ladybird's own [install rules](https://github.com/LadybirdBrowser/ladybird/blob/master/UI/cmake/InstallRules.cmake)
likewise package helpers and resources, with a directory bundle on macOS.
A single-file distribution would require an additional container or extraction
layer, or changes to how helpers and resources are loaded.

CI uses Rust from `rust-toolchain.toml` and Ladybird's pinned vcpkg versions with
Qt dependencies removed. It caches compiled dependencies and limits native build
parallelism to fit hosted runners. First builds can take hours. The checks cover
both Rust feature configurations and the real engine, then extract the archive
into a fresh directory, hide the engine build tree, and exercise rendering, JS
results, clipboard primitives, metadata, and session restoration without a
display. Packaging rejects unresolved libraries and Qt dependencies, includes
dependency license texts, and rewrites library paths for relocation.
Linux additionally exercises the packaged windowed client, palette, and OS
clipboard under Xvfb.

The build and packaging entrypoints are `scripts/ci/build-release` and
`python3 scripts/ci/package.py --platform linux-x64` (or `macos-arm64`). Run them
on the corresponding platform with the toolchain/dependencies installed by
`scripts/ci/install-dependencies`; that installer is intended for GitHub runners.
See `VALIDATION.md` for local checks versus checks awaiting an actual CI run.

## Rust API

`Browser` owns views independently of their presentation. It exposes `open`,
`navigate`, `back`, `forward`, `reload`, `stop`, `close`, `eval`, `set_load_script`,
`set_zoom`, `resize`, focus/input methods, `views`, and `read_frame`, along with
metadata, session snapshot/restore, and clipboard operations. These are
ordinary Rust calls; no protocol encoding or stdin/stdout is involved.

```rust,no_run
use bug::{Browser, BrowserOptions, Viewport};

fn main() -> Result<(), String> {
    let mut browser = Browser::new(BrowserOptions::default())?;
    let page = browser.open("https://ladybird.org")?;
    browser.resize(page, Viewport { width: 1280, height: 720, scale: 1.0 })?;
    browser.set_load_script(page, "document.documentElement.dataset.bug = 'yes'")?;
    // Repeat this part in your event loop:
    browser.poll();
    for event in browser.drain_events() {
        println!("{event:?}");
    }
    Ok(())
}
```

Call `poll` regularly on the initializing thread; drain typed `BrowserEvent`s
and use `read_frame` after `FrameReady` to obtain owned `0x00RRGGBB` pixels.
`read_frame` returns false before a frame exists. Resize is asynchronous: the
previous frame may remain available until the renderer supplies the new size.
`close` is asynchronous too; release presentation resources on `Closed`.

Ladybird supports one initialized `Browser` per process. Keep its operations and
destruction on the initializing thread. The library owns profile locking,
metadata, load hooks, and view lifetime; the client owns window handles and
presentation. A caller can use the library directly, or implement
`clients::Controller` for either supplied loop. `protocol::JsonController` is
just one controller implementation.

See [`examples/capture.rs`](examples/capture.rs) for a complete custom client
that captures a page without JSON or any windowing dependency. To depend on
Bug's core without its graphical client:

```toml
bug = { path = "/home/elf/dev/bug", default-features = false }
```

## Command palette

Press **Ctrl+Shift+P**, type to filter, then press Enter. Up/Down selects a result;
Esc goes back or closes the palette, and Ctrl+U clears its input. Commands cover
navigation, new window, history, reload, stop, close, zoom, and quit. Navigate and
New window prompt for a URL. Results can also be clicked.

The window client draws the palette over the page and calls `Browser` directly.
It does not modify the page DOM, and palette keyboard/pointer input is consumed
by the client. The rest of the time, keys go to the page. This initial palette
uses a bitmap font; unsupported display glyphs appear as `?`, while the entered
text is retained. Ctrl+V pastes into the palette; Ctrl+C/X copies/cuts its whole
input. IME composition is still pending.

## JavaScript results

`Browser::eval(view, script)` returns an `EvaluationId` immediately. Continue
pumping the browser and match `BrowserEvent::Evaluated { view, request, result }`.
The result is the script's completion value; promises are awaited. Requests queue
per view, and different views can execute independently. Closing a view cancels
its pending requests. Up to 128 evaluations may be pending at once.

The JSON adapter acknowledges the request, then emits the correlated result:

```json
{"cmd":"eval","view":1,"script":"Promise.resolve({title:document.title, links:document.links.length})"}
{"event":"evaluation_queued","view":1,"request":7}
{"event":"evaluated","view":1,"request":7,"result":{"type":"json","value":{"title":"Example","links":3}}}
```

`EvaluationResult::Json` carries a `serde_json::Value`. Top-level `undefined`,
bigints, and non-finite numbers have separate tags (`undefined`, `bigint`,
`number`); bigint/non-finite values are represented as strings. Other values use
JSON serialization semantics. Cycles and unsupported values produce errors,
rather than live object handles. Syntax errors, thrown exceptions, and rejected
promises return `type: "error"` with `name`, `message`, and optional `stack`.
Scripts execute in the page's main world, with its globals and built-ins.

The native execution path uses Ladybird's internal script executor, including
its default script timeout. This is asynchronous result delivery, not an extra
transport layer. A CPU-bound script can still block its renderer.

## View metadata and sessions

Every `ViewInfo` includes its URL, current title, viewport, zoom, and controller
metadata: a unique optional `name`, `tags`, and string-valued `properties`.
Use `set_metadata`/`find_view` in Rust, or:

```json
{"cmd":"set_metadata","view":1,"metadata":{"name":"mail","tags":["work"],"properties":{"workspace":"2"}}}
{"cmd":"find","name":"mail"}
{"cmd":"save_session","path":"/tmp/bug-session.json"}
```

`list` now includes metadata, viewport, and zoom. Names are independent of page
titles and runtime IDs. `find` replies with `found` and a view ID (or null).

`Browser::snapshot()` returns a versioned, serde-serializable `SessionSnapshot`.
It stores each view's URL, viewport, zoom, metadata, and load script. It does not
serialize a live DOM/JS heap, navigation history, clipboard, or WM layout.
Cookies and other profile data remain in the selected Ladybird profile.

`Browser::restore(&snapshot)` validates the snapshot and name uniqueness before
opening views, then returns their new runtime IDs in snapshot order. It appends
views; existing names cause an error. The JSON equivalents are `snapshot` and
`restore` (with a `session` object), or `save_session`/`load_session` with `path`.
Snapshots containing load scripts run those scripts as the reopened pages load.

```sh
target/debug/bug --session /tmp/bug-session.json
target/debug/bug --headless --session /tmp/bug-session.json
```

## Text clipboard

Ctrl+C, Ctrl+X, and Ctrl+V copy, cut, and paste page text using the OS clipboard;
Shift+Insert also pastes. X11 uses [arboard](https://docs.rs/arboard/latest/arboard/struct.Clipboard.html), and native Wayland uses
[smithay-clipboard](https://docs.rs/smithay-clipboard/latest/smithay_clipboard/struct.Clipboard.html) on the client's display connection. Clipboard ownership lasts
while the client is alive unless your desktop clipboard manager retains it.

The core API remains display-independent: `copy(view)` and `cut(view)` emit
`ClipboardChanged { text }`, `paste(view, text)` supplies text explicitly, and
`clipboard_text()` returns the most recently copied/cut/pasted text. Custom
clients choose their own clipboard provider. JSON commands are `copy`, `cut`,
and `paste` (with `view` and `text`); copy/cut produce a `clipboard` event.

This supports plain text. Image/rich-text clipboard content, primary-selection
integration, and page Clipboard API permission handling remain future work.
IME means **Input Method Editor**, the composition interface used for input such
as Japanese or Chinese; live composition is separate from Unicode clipboard paste.

## Headless views

```sh
target/debug/bug --headless --size 1280x720 https://ladybird.org
```

This uses real Ladybird layout, JavaScript, and rendering without creating OS
windows or connecting to X11/Wayland. It supports multiple views, renderer-created
new views, and the same JSON commands and events. `--size` sets the initial
physical viewport of every new view. Focus is logical in the headless client.

Capture a current frame through the scripting interface (PPM format):

```json
{"cmd":"resize","view":1,"width":1024,"height":768,"scale":1}
{"cmd":"screenshot","view":1,"path":"/tmp/bug.ppm"}
```

The screenshot command reports an error if no frame exists yet; wait for painting
and retry. A successful reply includes the actual width and height. It captures
page pixels, including in windowed mode; the host palette is not part of the page.
`--exit-after-load` exits after initial pages load, which can precede painting.

To build a binary without `winit`, `softbuffer`, or the palette font dependency:

```sh
cargo build --locked --no-default-features --target-dir target/headless
target/headless/debug/bug --headless https://ladybird.org
```

For a direct Rust capture, with an isolated profile:

```sh
cargo run --locked --no-default-features --example capture -- \
  "file://$PWD/examples/smoke.html" /tmp/bug.ppm /tmp/bug-capture-profile
```

## Current scope

Implemented: page-only windows, CPU frames, resize/scale/focus, pointer and basic
keyboard input, new-window handling, navigation/history/reload, zoom, JS execution,
load scripts, a command palette, headless views, frame capture, and JSON event
output through the shared Rust library API.

This is an initial prototype. GPU presentation, IME composition,
accessibility, file/select/color pickers, controller-driven permission
and dialog replies, and complete download integration remain to be implemented.
Alerts emit an event and close; confirms/prompts and permission requests are
dismissed. Before-unload requests are currently accepted. These defaults are
explicit prototype behavior, not a finished policy system.

Load scripts execute in the page's main world after a top-level load finishes.
They are not isolated extensions or document-start scripts. Ladybird's engine event loop is polled
without blocking every 8 ms; integrating its wakeups with the host is future work.

## Scripting

Commands are JSON objects, one per line. Events use the same framing on stdout;
diagnostics go to stderr. No command text is evaluated by a shell.

```json
{"cmd":"open","url":"https://example.org"}
{"cmd":"navigate","view":1,"url":"https://ladybird.org"}
{"cmd":"back","view":1}
{"cmd":"eval","view":1,"script":"document.body.style.background = 'ivory'"}
{"cmd":"set_load_script","view":1,"script":"document.documentElement.dataset.bug = 'yes'"}
{"cmd":"list"}
```

Extension hooks mean your own code acting on events and injected page scripts;
they do not mean Chrome/Firefox WebExtension compatibility.

Other commands are `forward`, `reload`, `stop`, `close`, and `focus` (each with
`view`), `zoom` (with `view` and `factor`, 0.25–5), and `quit`. IDs come from
`opened` events; don't assume they start at any particular value. `list` produces
a `views` event containing IDs, titles, and URLs. Events include `ready`, `opened`,
`closed`, `focused`, `url`, `title`, `loaded`, `resized`, `screenshot`, `hook`, and
`error`. Native hooks carry engine descriptions; `kind: "palette"` reports
`opened`/`closed` in the windowed client. JSON strings are data, never shell code.

For a simple wmenu integration, connect stdin to a FIFO of your choosing:

```sh
control=$(mktemp -d "${XDG_RUNTIME_DIR:-/tmp}/bug.XXXXXX")
mkfifo -m 600 "$control/commands"
# Open read/write so command writers can come and go without EOF stopping input.
target/debug/bug 0<>"$control/commands" >"$control/events.jsonl" &
echo "$control"
```

Bind `examples/wmenu-open /path/printed/above/commands` in your WM. The FIFO is an
optional external controller transport; it adds no broker between Rust and C++.
Controllers can instead launch Bug directly with ordinary subprocess pipes.
Stdin EOF stops command input without closing windows. Remove the control directory
after the process exits. Window focusing on Wayland is subject to compositor policy;
your WM's own focus commands are preferable.

## Checks

```sh
nix-shell --run 'cargo test --locked && cargo clippy --locked --all-targets -- -D warnings'
nix-shell --run 'python3 tests/render.py && tests/no-qt'
nix-shell --run 'env -u DISPLAY -u WAYLAND_DISPLAY cargo test --locked --no-default-features --test library -- --ignored'
nix-shell --run 'cargo build --locked && python3 tests/headless.py'
nix-shell --run 'python3 tests/features.py'
nix-shell --arg withTests true --run 'xvfb-run -a -s "-screen 0 1280x1024x24" python3 tests/smoke.py'
nix-shell --arg withTests true --run 'python3 tests/wayland.py'
```

The smoke test requires a built engine and `cargo build`. It exercises the real
Ladybird renderer through the Rust host on a virtual display, without opening
windows on your desktop. Unit tests exercise command validation and callback
ownership. The Wayland test starts its own headless Weston compositor, with no
connection to your desktop. See `VALIDATION.md` for the checks actually run on
this checkout.

Ladybird and its dependencies retain their upstream licenses; see
`vendor/ladybird/LICENSE` after preparing the engine. Bug's code uses BSD-2-Clause.
