# Bug

A programmable, page-only host for Ladybird. **kytin** is an alternate project name.

One page per ordinary window, or an offscreen view with no display server. Your
tiling window manager owns layout and switching; your Rust code or scripts own
navigation and interaction. No tabs, address bar, toolbar, or Qt. Ctrl+Shift+P
opens a transient command palette in the windowed client.
macOS also accepts Command+Shift+P and Command+C/X/V for text clipboard actions.

## Start here

Extract a [release](https://github.com/elfstead/bug/releases), put its `bin/` directory on
your PATH, and open a URL. The first invocation hosts the browser. Subsequent
invocations open windows in that profile.

```sh
bug https://example.org >/dev/null
```

Once the window opens, use a second shell to create and address a specific view.

```sh
view=$(bug open --wait https://example.org)
bug eval --view "$view" 'document.title'
bug views --json
```

With wmenu installed, add this to your Sway config. Both executables must be on
Sway's PATH. The shipped helper prompts for a URL and works on the first launch
and on later ones.

```sway
bindsym $mod+g exec wmenu-open
```

`bug help` lists interactive actions. `bug help eval` explains input, output and
target selection. Commands such as `bug open`, `bug views` and `bug eval` require a
running host and exit 1 with a diagnostic if the command fails. `bug open` returns
a new view ID as soon as it exists. `--wait` waits for its first top-level load.
`bug watch` streams view notifications as JSON lines. `bug eval --focused` targets
the focused page. Use `--last-focused` after switching to a terminal or launcher.
Headless hosts acquire logical focus via the `focus` JSON command. Use `--view ID`
for automation that must keep targeting the same page.

Put `--profile DIR` before a command to select an instance. A user-private Unix
socket connects CLI invocations to that profile's host. `bug socket` prints its
path. The Rust API continues calling Ladybird directly.

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

### Changing the native interface

Edit `native/abi.toml` to add an exported function, callback signature, event ID,
or action ID. Run `scripts/generate-abi` with Python 3.11+ and commit both
outputs. C++ includes `native/bug.h` and Rust loads the symbols declared in
`src/engine/abi.rs`. The checked-in files let either side build independently.
CI runs `scripts/generate-abi --check` before the platform builds.

Keep existing numeric IDs stable. Bump the ABI version when either side
requires a different contract, including a new required symbol. Implement the
Ladybird callback behavior in `native/bridge.cpp` and expose it to clients
through the Rust API.

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
with ABI version 4. Rebuild the engine after updating an older checkout.
`--help` works without building the engine.

The Nix shell uses your `<nixpkgs>`; Ladybird and Cargo dependencies are pinned,
but the Nix package collection currently follows your configured channel. The
shell reuses the Ladybird package's engine dependencies and explicitly removes
its Qt inputs. It uses `curlFull` for WebSocket support. If an older Nix build
cached plain curl, run `scripts/build-engine -U '*CURL*'` once to refresh it.
`patches/frontend.patch` selects Bug instead of `UI/Qt`, uses
thread-local storage compatible with dynamic loading, supports system FFmpeg
outside vcpkg, and permits newer Rust compiler warnings. Preparation refuses
to overwrite an unexpected Ladybird revision or incompatible local edits.
`patches/api.patch` makes Ladybird's internal script-completion method virtual,
allowing the adapter to receive execution results through existing IPC. It does
not start a WebDriver server or add a socket.
`patches/controller-hooks.patch` passes a per-view script to the renderer over
Ladybird’s page IPC and runs it before the top-level document starts its parser.
It also enables explicitly requested before-unload prompts during automation.

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
Qt and the official macOS updater dependencies removed. It caches compiled dependencies and limits native build
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

Press **Ctrl+Shift+P** (or **Command+Shift+P** on macOS), type to filter, then press Enter. Up/Down selects a result;
Esc goes back or closes the palette, and Ctrl+U clears its input. Commands cover
navigation, new window, history, reload, stop, close, zoom, JavaScript evaluation, listing views, and quit. Navigate and
New window prompt for a URL. Results can also be clicked.

The window client draws the palette over the page and calls `Browser` directly.
It does not modify the page DOM, and palette keyboard/pointer input is consumed
by the client. The rest of the time, keys go to the page. The palette shapes text using installed fonts and glyph fallback. Ctrl+V pastes into the palette; Ctrl+C/X copies/cuts its whole
input. IME composition is still pending.
On macOS, Command+V and Command+C/X perform the same palette operations.

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
Persistent cookies and localStorage survive a clean restart with the same profile.
The pinned Ladybird engine keeps IndexedDB records in memory. Sites that use
IndexedDB for login state can require a fresh sign-in after a restart.

`Browser::restore(&snapshot)` validates the snapshot and name uniqueness before
opening views, then returns their new runtime IDs in snapshot order. It appends
views; existing names cause an error. The JSON equivalents are `snapshot` and
`restore` (with a `session` object), or `save_session`/`load_session` with `path`.
Snapshots containing load scripts run those scripts as the reopened pages load.

```sh
target/debug/bug --session /tmp/bug-session.json
target/debug/bug --headless --session /tmp/bug-session.json
```

## Controller callbacks

Rust controllers receive notifications through `Controller::event` or
`Browser::drain_events`. The JSON stream includes all notifications by default.
Use `subscribe` to replace the notification list, or pass `null` to restore all.
Replies, errors and requests that need an answer are always delivered.

```json
{"cmd":"subscribe","events":["opened","closed","loaded"]}
{"cmd":"set_request_handlers","view":1,"kinds":["confirm","file_picker"]}
{"event":"request","view":1,"request":7,"kind":"confirm","message":"Discard draft?"}
{"cmd":"reply_request","view":1,"request":7,"reply":{"kind":"confirm","accept":false}}
```

Register handlers before navigating if a page might ask during its first
script. Each registration replaces the list for that view. Match both the
view and request ID when replying. Handle `request_cancelled` by dismissing
any outstanding UI for that ID. Removing a handler or closing JSON stdin
settles its pending requests using the default response.

| Request kind | Fields on the request | Fields in the reply alongside `kind` |
| --- | --- | --- |
| `alert` | `message` | None |
| `confirm` | `message` | `accept` (boolean) |
| `prompt` | `message`, `default` | `text` (string, or null to cancel) |
| `before_unload` | `source` | `leave` (boolean) |
| `permission` | `name`, `origin`, `is_file`, `system_denied`, `show_os_hint` | `decision`: `dismiss`, `block`, `allow_once`, or `allow_always`. Use `dismiss` for a system-denied notice. |
| `file_picker` | `multiple`, `accept` | `paths` (array of local paths, empty to cancel) |

In Rust, use `Browser::set_request_handlers`, match `BrowserEvent::Request`,
and call `Browser::reply_request` with a typed `RequestReply`. A registered
file picker gives the controller sole responsibility for the chooser.
An evaluation interrupted by a modal returns a `UserPromptOpened` error.
The page script resumes after the controller answers.

## Document-start scripts

Supply `document_start_script` when opening a view to cover its first navigation.
Use `set_document_start_script` to replace it for subsequent navigations,
or pass an empty string to clear it. The script runs in the main JavaScript
world of each newly loaded top-level document before inline page scripts.
At that point the page body may not exist. A load script can still use the
completed DOM. Session snapshots preserve both scripts.

```json
{"cmd":"open","url":"https://example.org","document_start_script":"window.addEventListener('DOMContentLoaded', () => console.log(document.title))"}
{"cmd":"set_document_start_script","view":1,"script":""}
```

The corresponding Rust methods are `open_with_document_start_script`,
`set_document_start_script`, and `set_load_script`.

## Keyboard and pointer input

`key` and `mouse` use the same Rust input path as the window client. Supply one
key event per press or release. `name` is a Ladybird key label and `codepoint`
is the Unicode scalar to insert on press, or zero for a non-text key.
Modifier bits are Alt 1, Ctrl 2, Shift 4, and Super 8.

```json
{"cmd":"key","view":1,"name":"Escape","pressed":true}
{"cmd":"key","view":1,"name":"Escape","pressed":false}
{"cmd":"mouse","view":1,"kind":"wheel","x":200,"y":160,"dy":120}
```

Mouse kinds are `move`, `down`, `up`, `wheel`, and `leave`. Coordinates are
physical viewport pixels. Optional fields are `button`, `buttons`, `modifiers`,
`dx`, `dy`, and `clicks`. Each defaults to zero. Button masks are primary 1,
secondary 2, middle 4, back 8, and forward 16.

## File inputs

By default, a page opening an HTML file input produces a `BrowserEvent::FilePickerRequested`.
Its request ID, view ID, multiple-selection flag, and HTML `accept` hints are
available to every client. The windowed client opens a desktop chooser (the
desktop portal on Linux or AppKit on macOS). It builds desktop filters from extensions, MIME types and media wildcards.
Headless clients choose how to answer. Only supply local paths selected by
the user or a trusted controller.

```json
{"cmd":"click","view":1,"x":20,"y":16}
{"event":"file_picker_requested","view":1,"request":3,"multiple":false,"accept":[".txt"]}
{"cmd":"reply_file_picker","view":1,"request":3,"paths":["/home/me/notes.txt"]}
```

The click uses physical viewport coordinates, including in headless mode.
In Rust, call `Browser::reply_file_picker(view, request, &paths)`. An empty
slice cancels. An invalid selection can be retried. Replies to expired or
already answered requests return `Ok(false)`. The page receives ordinary
HTML files with basenames and contents.

## Text clipboard

Ctrl+C, Ctrl+X, and Ctrl+V copy, cut, and paste page text using the OS clipboard.
macOS additionally supports its usual **Command+C/X/V** shortcuts. Shift+Insert
also pastes. X11 and macOS use [arboard](https://docs.rs/arboard/latest/arboard/struct.Clipboard.html);
its macOS backend reads and writes Apple's `NSPasteboard`. Native Wayland uses
[smithay-clipboard](https://docs.rs/smithay-clipboard/latest/smithay_clipboard/struct.Clipboard.html)
on the client's display connection. On X11/Wayland, clipboard ownership lasts
while the client is alive unless a clipboard manager retains it; macOS retains
the copied text in the system pasteboard after the writer closes.

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
accessibility, select/color pickers, and complete download integration remain to be implemented.
macOS has a clipboard implementation, but full window/input/HiDPI parity has
not yet been verified on a Mac. CI tests the native clipboard against `pbcopy`
and `pbpaste`; that does not replace testing Command shortcuts in a live window.
By default, alerts close, confirms and prompts are dismissed, permission requests
are dismissed, and before-unload requests are accepted. Controllers can take
over any of these decisions per view. Registering `before_unload` also enables
the prompt after an evaluation has initialized Ladybird’s script executor.

TODO: persist IndexedDB transactions and verify records and non-exportable
test keys across a restart. The pinned renderer currently holds them in memory.

Document-start and after-load scripts share the page’s main world. Isolated
extension worlds remain future work. The host polls Ladybird’s event loop
every 8 ms.

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

### Replies and subscriptions

Give a command a numeric `request_id` to receive a single completion record.
The caller chooses an ID unique among its outstanding commands on that connection.
For example, this exchange opens a view and evaluates on exactly that view:

```jsonl
{"cmd":"open","url":"about:blank","wait_for_load":true,"request_id":7}
{"reply_to":7,"result":{"view":3}}
{"cmd":"eval","view":3,"script":"6 * 7","request_id":8}
{"reply_to":8,"result":{"type":"json","value":42}}
```

A failed command returns `reply_to` and `error.message`. An evaluation exception
also includes its JavaScript error name. Notifications such as `opened` and
`loaded` have an `event` field and may arrive between replies. A view argument
accepts a numeric ID, `"focused"`, or `"last_focused"`.
Commands without `request_id` on stdin keep their original event-based behavior.

Use ordinary subprocess pipes to drive a child Bug. For an existing instance,
connect to the Unix socket printed by `bug socket`. Send one JSON command per
line with a request ID and keep the connection open while awaiting replies.
Socket clients initially receive only replies. Send `subscribe` to request view
notifications. Each view's delegated callbacks belong to one controller at a time.
Disconnecting releases its handlers and settles outstanding prompts. A slow
reader has a bounded output queue (256 records and 16 MiB); exceeding either
limit disconnects its controller. After reconnecting, request `list` for current
state. Stdin EOF releases delegated handlers and leaves windows open.

## Checks

```sh
nix-shell --run 'cargo test --locked && cargo clippy --locked --all-targets -- -D warnings'
nix-shell --run 'python3 tests/render.py && tests/no-qt'
nix-shell --run 'env -u DISPLAY -u WAYLAND_DISPLAY cargo test --locked --no-default-features --test library -- --ignored'
nix-shell --run 'cargo build --locked && python3 tests/headless.py'
nix-shell --run 'python3 tests/features.py && python3 tests/cli.py'
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
