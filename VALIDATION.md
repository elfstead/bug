# Validation

Run on 2026-11-06 on x86_64 Linux with the repository's Nix shell.
Ladybird revision: `7786ab9c13f000a7f9fcd75248ee10d691a29195`.

## Passed

- Full Release build of the C++ adapter, Ladybird libraries, and helper processes.
  Built with `scripts/build-engine`; resumed with
  `cmake --build build/engine --target bug_engine --parallel 12` to increase
  parallelism during the first build.
- `cargo build --locked` and `cargo test --locked` (five unit tests; the native
  integration test is opt-in and was run separately below).
- `cargo clippy --locked --all-targets -- -D warnings`, also with
  `--no-default-features`.
- `python3 tests/render.py`: loaded the local HTML fixture in the real engine,
  obtained an 800×600 CPU frame, and verified the expected background and rectangle
  colors. The rendered text and controls were also visually inspected in
  `build/smoke.png` (converted from `build/smoke.ppm`).
- `tests/no-qt`: checked the Rust executable, native adapter, and all built helper
  executables with `ldd`; no Qt dependencies or missing shared libraries.
- `xvfb-run -a -s "-screen 0 1280x1024x24" python3 tests/smoke.py`: actual X11 window, page load, JavaScript
  execution, pointer click, keyboard text input, a reload script, a link-created
  window, explicit new view, view listing, close, and clean shutdown. Also tested
  Ctrl+Shift+P, filtering/dismissal without text leaking into the page's focused
  input, reload through the palette, and the palette's New window URL prompt.
  Captured actual X11 presentation pixels and asserted the palette colors;
  visually inspected `build/palette.png`.
- `python3 tests/wayland.py`: native Wayland window under a separate headless
  Weston compositor, page load, JS title update, presentation without reported
  errors, and clean shutdown. `DISPLAY` was removed, so this did not use XWayland.
- Public HTTPS navigation to `https://ladybird.org/` through the Rust host under
  Xvfb: observed title `Ladybird`, load completion, and exit status 0. This
  exercised the real network helper and TLS path with sandboxing enabled.
- Shell syntax checks for build/preparation scripts, the wmenu example, and the
  dependency check; Python syntax checks for the test scripts.

## Rust API and headless clients

- `env -u DISPLAY -u WAYLAND_DISPLAY cargo test --locked --no-default-features
  --test library -- --ignored --nocapture`: exercised the public Rust API directly,
  without JSON or a window client. Checked profile exclusion, real rendered page
  pixels, resize, invalid arguments, zoom, keyboard input, JavaScript, load scripts,
  multiple views, asynchronous close, and state removal.
- `python3 tests/headless.py`: real headless CLI with both display variables
  removed, frame capture and expected colors, resizing, JS, load scripts,
  multiple views, logical focus, and automatic exit on closing the last view.
  Also passed with `BUG_TEST_BINARY` pointing to `target/headless/debug/bug`.
- `cargo build --locked --no-default-features --target-dir target/headless`:
  built a CLI with no graphical client. `cargo tree --no-default-features
  --edges normal` confirmed that winit, softbuffer, and font8x8 were absent.
- Ran `examples/capture.rs` with `--no-default-features` and both display
  variables removed. This independent Rust client, with its own event loop,
  wrote a 1024×768 page image to `build/example-capture.ppm`.

## JavaScript results, metadata, and clipboard

- Rebuilt native bridge ABI 2 and LibWebView after adding the script-completion
  override and clipboard actions. The upstream change is in `patches/api.patch`.
- `tests/features.py`: actual results for numbers, objects, promises, undefined,
  bigint, and NaN; syntax errors, thrown exceptions, rejected promises, cyclic
  serialization errors, concurrent request correlation, and close cancellation.
- The same test checked Unicode copy/cut/paste through the library's editing
  path, named metadata lookup, duplicate-name and invalid-session rejection,
  serialization to disk, and fresh-process restoration of metadata, viewport,
  zoom, and load scripts.
- The X11 smoke test copied to and pasted from an external `xclip` process,
  including Japanese text and emoji. Also checked cut and pasting a command into
  the palette. The native Wayland startup/rendering regression passed with the
  clipboard backend initialized; interactive Wayland clipboard transfer still
  needs testing on a compositor with an input seat.

## Release workflow and portability

- Added `.github/workflows/release.yml` for Ubuntu 24.04 x64 and macOS 15 arm64.
  Pushes to `master` publish commit-addressed snapshot prereleases after tests;
  explicit version tags designate milestones and manual branch runs upload
  artifacts. No release was published from this checkout.
- Passed `actionlint` and `shellcheck` on the workflow and release/build scripts;
  Python packaging scripts passed bytecode compilation and static checking.
- Passed Rust formatting, unit tests, and Clippy with warnings denied, with and
  without the windowed feature after making clipboard dependencies platform-specific.
- Reconfigured and built the native Linux engine with the revised patches.
  Passed the real Rust API integration and Python headless/features tests.
- A standalone CMake fixture exercising the macOS resource branch copied fonts,
  themes, and page assets into `Resources` without an application bundle.
- The Linux windowed smoke test passed after rebuilding the default-feature
  executable: real X11 windows, pointer/keyboard input, palette rendering and
  commands, and external Unicode clipboard transfer. The preceding headless
  integration test had replaced the debug executable with a headless-only one;
  release CI uses its separately built `target/release/bug` archive.
- Copied the existing Linux host, helpers, libraries, and resources into a
  temporary archive. The archive smoke test extracted it under a path with
  spaces and hid `build/engine`; rendering, JS results, clipboard primitives,
  metadata, and fresh-process session restoration passed. This checks installed
  engine discovery and resource/helper relocation. It uses existing Nix runtime
  libraries and does **not** establish distro portability of a release archive.
- Production CI builds its dependencies with Ladybird's pinned vcpkg manifest.
  Its packaging step audits and relocates the dependency closure, then tests
  the extracted archive without library-path overrides. Linux also runs the
  windowed smoke test under Xvfb.
- Full vcpkg release builds, macOS compilation/signing/runtime behavior, and
  GitHub publication still await the first CI run. macOS archives are ad-hoc
  signed, not Developer ID notarized.
- Snapshot identities use the first 12 characters of the source commit;
  archive metadata retains the full commit. Four release identity tests cover
  branch/local snapshots, alpha prereleases, explicit stable tags, and rejection
  of invalid tag names. Workflow/shell/Python checks pass. Both Rust feature
  configurations pass unit tests and Clippy after adding `bug --version` and
  the public build identity constants.

## Limits

These are prototype integration checks, not browser conformance or security
validation. Pointer and keyboard behavior was exercised on X11; native Wayland
input, HiDPI, and integration with a particular tiling compositor still need
interactive testing. The Wayland test checks successful presentation calls,
not compositor screenshot contents.

The tests emitted a Fontconfig configuration warning on this machine; page text
still rendered correctly. An initial public-page check using `example.org`
failed because the host resolver could not resolve it (also reproduced with
`curl` and Python's `socket.getaddrinfo`); the subsequent Ladybird-site check
passed. The Nix package collection follows the machine's `<nixpkgs>` channel
rather than a repository lock file.

See README.md for feature gaps and current dialog/permission defaults.
