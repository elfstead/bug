use crate::engine::{Engine, MouseInput};
use crate::evaluation::{Evaluation, NativeEvaluation};
use crate::{EvaluationId, EvaluationResult, SavedView, SessionSnapshot, ViewMetadata};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

pub type ViewId = u64;

fn default_engine_path() -> PathBuf {
    let name = format!(
        "{}bug_engine{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    // Release archives keep the host and helpers in bin/ and the adapter in lib/.
    if let Ok(executable) = std::env::current_exe()
        && let Some(prefix) = executable.parent().and_then(Path::parent)
    {
        let bundled = prefix.join("lib").join(&name);
        if bundled.is_file() {
            return bundled;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("build/engine/lib")
        .join(name)
}

/// A physical pixel viewport and its device scale. Headless views use this too.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}
impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            scale: 1.0,
        }
    }
}
impl Viewport {
    pub fn validate(self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width > 16384
            || self.height > 16384
            || u64::from(self.width) * u64::from(self.height) > 64 * 1024 * 1024
            || !self.scale.is_finite()
            || !(0.25..=8.).contains(&self.scale)
        {
            return Err(
                "invalid viewport: dimensions 1–16384, at most 64M pixels, scale 0.25–8".into(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct BrowserOptions {
    pub engine: PathBuf,
    pub profile: PathBuf,
    pub viewport: Viewport,
}
impl Default for BrowserOptions {
    fn default() -> Self {
        Self {
            viewport: Viewport::default(),
            engine: default_engine_path(),
            profile: std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
                })
                .join("bug/profile"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ViewInfo {
    pub id: ViewId,
    pub url: String,
    pub title: String,
    pub viewport: Viewport,
    pub zoom: f64,
    pub metadata: ViewMetadata,
}

/// Library events contain no window-system handles or transport framing.
#[derive(Debug, Clone)]
pub enum BrowserEvent {
    Evaluated {
        view: ViewId,
        request: EvaluationId,
        result: EvaluationResult,
    },
    MetadataChanged {
        view: ViewId,
        metadata: ViewMetadata,
    },
    ClipboardChanged {
        text: String,
    },
    Opened {
        view: ViewId,
    },
    Closed {
        view: ViewId,
    },
    Title {
        view: ViewId,
        title: String,
    },
    Url {
        view: ViewId,
        url: String,
    },
    Loaded {
        view: ViewId,
        url: String,
    },
    FrameReady {
        view: ViewId,
    },
    ViewportChanged {
        view: ViewId,
        viewport: Viewport,
    },
    FocusRequested {
        view: ViewId,
    },
    Focused {
        view: ViewId,
    },
    Hook {
        view: ViewId,
        kind: String,
        data: String,
    },
    Error {
        message: String,
    },
}

/// Owned CPU pixels packed as 0x00RRGGBB, row-major, with no row padding.
#[derive(Debug, Default)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}
impl Frame {
    /// Write a portable RGB image without a display server or image encoder.
    pub fn write_ppm(&self, path: impl AsRef<Path>) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.pixels.len() != self.width as usize * self.height as usize
        {
            return Err("frame is empty or has inconsistent dimensions".into());
        }
        let mut out = std::io::BufWriter::new(File::create(path).map_err(|e| e.to_string())?);
        writeln!(out, "P6\n{} {}\n255", self.width, self.height).map_err(|e| e.to_string())?;
        for pixel in &self.pixels {
            out.write_all(&[(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8])
                .map_err(|e| e.to_string())?;
        }
        out.flush().map_err(|e| e.to_string())
    }
}

/// Key names and modifier bits follow native/bug.h. Text is one Unicode scalar;
/// IME composition is not supported yet. This type does not depend on winit.
#[derive(Debug)]
pub struct KeyInput<'a> {
    pub pressed: bool,
    pub name: &'a str,
    pub codepoint: u32,
    pub modifiers: u32,
    pub repeat: bool,
}

struct ViewState {
    info: ViewInfo,
    load_script: String,
}

/// Owns view lifetimes, state, scripts, and the profile lock. Construct, operate,
/// poll and drop on the same thread. Only one Browser may be initialized per
/// process, matching Ladybird's process-wide state. No display server is needed.
pub struct Browser {
    engine: Engine,
    views: BTreeMap<ViewId, ViewState>,
    events: VecDeque<BrowserEvent>,
    default_viewport: Viewport,
    next_evaluation: EvaluationId,
    evaluations: BTreeMap<EvaluationId, Evaluation>,
    active_evaluations: BTreeMap<ViewId, EvaluationId>,
    clipboard: String,
    // Keep this after engine so shutdown completes before the lock is released.
    _profile_lock: File,
}
impl Browser {
    pub fn new(options: BrowserOptions) -> Result<Self, String> {
        options.viewport.validate()?;
        let library = options.engine.canonicalize().map_err(|e| {
            format!(
                "engine not built at {}: {e}. Run scripts/build-engine first.",
                options.engine.display()
            )
        })?;
        std::fs::create_dir_all(&options.profile).map_err(|e| e.to_string())?;
        let profile = options.profile.canonicalize().map_err(|e| e.to_string())?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(profile.join(".bug.lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock().map_err(|e| {
            format!(
                "profile {} is already in use or cannot be locked: {e}",
                profile.display()
            )
        })?;
        let executable = library
            .parent()
            .and_then(|p| p.parent())
            .ok_or("engine path must be under lib/")?
            .join("bin/bug");
        Ok(Self {
            engine: Engine::load(&library, &executable, &profile)?,
            views: BTreeMap::new(),
            events: VecDeque::new(),
            default_viewport: options.viewport,
            next_evaluation: 1,
            evaluations: BTreeMap::new(),
            active_evaluations: BTreeMap::new(),
            clipboard: String::new(),
            _profile_lock: lock,
        })
    }

    /// Pump Ladybird without blocking. Call regularly from your own event loop.
    pub fn poll(&mut self) {
        self.engine.poll();
        self.collect_events();
        self.dispatch_evaluations();
    }
    pub fn drain_events(&mut self) -> impl Iterator<Item = BrowserEvent> + '_ {
        self.events.drain(..)
    }
    pub fn views(&self) -> impl Iterator<Item = &ViewInfo> {
        self.views.values().map(|v| &v.info)
    }
    /// Default for subsequently opened views, including renderer-created ones.
    /// Existing view sizes are unaffected.
    pub fn set_default_viewport(&mut self, viewport: Viewport) -> Result<(), String> {
        viewport.validate()?;
        self.default_viewport = viewport;
        Ok(())
    }
    pub fn view(&self, id: ViewId) -> Result<&ViewInfo, String> {
        self.views
            .get(&id)
            .map(|v| &v.info)
            .ok_or_else(|| format!("unknown view {id}"))
    }
    pub fn open(&mut self, url: &str) -> Result<ViewId, String> {
        let id = self.engine.open(url)?;
        self.collect_events();
        if let Some(state) = self.views.get_mut(&id)
            && state.info.url.is_empty()
        {
            state.info.url = url.into();
        }
        Ok(id)
    }
    pub fn navigate(&mut self, view: ViewId, url: &str) -> Result<(), String> {
        self.engine.navigate(view, url)
    }
    pub fn back(&mut self, view: ViewId) -> Result<(), String> {
        self.engine.action(view, 1, "", 0.)
    }
    pub fn forward(&mut self, view: ViewId) -> Result<(), String> {
        self.engine.action(view, 2, "", 0.)
    }
    pub fn reload(&mut self, view: ViewId) -> Result<(), String> {
        self.engine.action(view, 3, "", 0.)
    }
    pub fn stop(&mut self, view: ViewId) -> Result<(), String> {
        self.engine.action(view, 4, "", 0.)
    }
    /// Closure is asynchronous. Keep client resources until the Closed event.
    pub fn close(&mut self, view: ViewId) -> Result<(), String> {
        self.engine.action(view, 5, "", 0.)?;
        self.cancel_evaluations(view, "view is closing");
        Ok(())
    }
    /// Evaluate a script's completion value in the page, awaiting promises.
    /// Results arrive in Evaluated events. Requests are queued per view because
    /// Ladybird's internal script executor has one active execution per page.
    pub fn eval(&mut self, view: ViewId, script: &str) -> Result<EvaluationId, String> {
        self.view(view)?;
        if script.len() > 1024 * 1024 || script.contains('\0') {
            return Err("script exceeds 1 MiB or contains a NUL byte".into());
        }
        if self.evaluations.len() >= 128 {
            return Err("too many pending evaluations (limit 128)".into());
        }
        let request = self.next_evaluation;
        self.next_evaluation = request.checked_add(1).ok_or("evaluation IDs exhausted")?;
        self.evaluations.insert(
            request,
            Evaluation {
                view,
                script: script.into(),
            },
        );
        self.dispatch_evaluations();
        Ok(request)
    }
    pub fn copy(&mut self, view: ViewId) -> Result<(), String> {
        self.engine.action(view, 10, "", 0.)
    }
    pub fn cut(&mut self, view: ViewId) -> Result<(), String> {
        self.engine.action(view, 11, "", 0.)
    }
    /// Explicitly paste supplied text. Clients obtain it from their clipboard.
    pub fn paste(&mut self, view: ViewId, text: &str) -> Result<(), String> {
        self.engine.action(view, 12, text, 0.)?;
        self.clipboard = text.into();
        Ok(())
    }
    /// Last copied/cut/pasted text, also usable by a headless controller.
    pub fn clipboard_text(&self) -> &str {
        &self.clipboard
    }

    pub fn find_view(&self, name: &str) -> Option<ViewId> {
        self.views()
            .find(|v| v.metadata.name.as_deref() == Some(name))
            .map(|v| v.id)
    }
    pub fn set_metadata(&mut self, view: ViewId, metadata: ViewMetadata) -> Result<(), String> {
        self.view(view)?;
        metadata.validate()?;
        if let Some(name) = &metadata.name
            && self.find_view(name).is_some_and(|id| id != view)
        {
            return Err(format!("view name already exists: {name}"));
        }
        self.views.get_mut(&view).unwrap().info.metadata = metadata.clone();
        self.events
            .push_back(BrowserEvent::MetadataChanged { view, metadata });
        Ok(())
    }
    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            version: 1,
            views: self
                .views
                .values()
                .map(|v| SavedView {
                    url: v.info.url.clone(),
                    viewport: v.info.viewport,
                    zoom: v.info.zoom,
                    metadata: v.info.metadata.clone(),
                    load_script: v.load_script.clone(),
                })
                .collect(),
        }
    }
    /// Validate first, then append reopened views. IDs are fresh, in snapshot
    /// order. Existing views are retained; duplicate stable names are rejected.
    pub fn restore(&mut self, snapshot: &SessionSnapshot) -> Result<Vec<ViewId>, String> {
        snapshot.validate()?;
        for saved in &snapshot.views {
            if let Some(name) = &saved.metadata.name
                && self.find_view(name).is_some()
            {
                return Err(format!("view name already exists: {name}"));
            }
        }
        let mut ids = Vec::new();
        for saved in &snapshot.views {
            let id = self.open(&saved.url)?;
            self.resize(id, saved.viewport)?;
            self.set_zoom(id, saved.zoom)?;
            self.set_load_script(id, &saved.load_script)?;
            self.set_metadata(id, saved.metadata.clone())?;
            ids.push(id);
        }
        Ok(ids)
    }

    fn dispatch_evaluations(&mut self) {
        let ready: Vec<_> = self
            .evaluations
            .iter()
            .filter(|(_, e)| !self.active_evaluations.contains_key(&e.view))
            .map(|(id, _)| *id)
            .collect();
        for request in ready {
            let evaluation = &self.evaluations[&request];
            let view = evaluation.view;
            if self.active_evaluations.contains_key(&view) {
                continue;
            }
            self.active_evaluations.insert(view, request);
            if let Err(message) = self.engine.evaluate(
                view,
                request,
                &crate::evaluation::function_body(&evaluation.script),
            ) {
                self.active_evaluations.remove(&view);
                self.evaluations.remove(&request);
                self.events.push_back(BrowserEvent::Evaluated {
                    view,
                    request,
                    result: EvaluationResult::Error {
                        name: "EngineError".into(),
                        message,
                        stack: None,
                    },
                });
            }
        }
    }
    fn cancel_evaluations(&mut self, view: ViewId, reason: &str) {
        self.active_evaluations.remove(&view);
        let requests: Vec<_> = self
            .evaluations
            .iter()
            .filter(|(_, e)| e.view == view)
            .map(|(id, _)| *id)
            .collect();
        for request in requests {
            self.evaluations.remove(&request);
            self.events.push_back(BrowserEvent::Evaluated {
                view,
                request,
                result: EvaluationResult::cancelled(reason),
            });
        }
    }
    pub fn set_zoom(&mut self, view: ViewId, factor: f64) -> Result<(), String> {
        self.engine.action(view, 7, "", factor)?;
        self.views.get_mut(&view).ok_or("missing view")?.info.zoom = factor;
        Ok(())
    }
    pub fn set_load_script(&mut self, view: ViewId, script: &str) -> Result<(), String> {
        if script.contains('\0') {
            return Err("script contains a NUL byte".into());
        }
        self.views
            .get_mut(&view)
            .ok_or_else(|| format!("unknown view {view}"))?
            .load_script = script.into();
        Ok(())
    }
    /// Ask the client to focus a view. A headless client gives it logical focus.
    pub fn request_focus(&mut self, view: ViewId) -> Result<(), String> {
        self.view(view)?;
        self.events.push_back(BrowserEvent::FocusRequested { view });
        Ok(())
    }
    /// Report actual focus from the client to the renderer.
    pub fn set_focused(&mut self, view: ViewId, focused: bool) -> Result<(), String> {
        self.engine
            .action(view, if focused { 8 } else { 9 }, "", 0.)?;
        if focused {
            self.events.push_back(BrowserEvent::Focused { view });
        }
        Ok(())
    }
    pub fn resize(&mut self, view: ViewId, viewport: Viewport) -> Result<(), String> {
        viewport.validate()?;
        self.engine
            .resize(view, viewport.width, viewport.height, viewport.scale)?;
        let state = self.views.get_mut(&view).ok_or("missing view")?;
        if state.info.viewport != viewport {
            state.info.viewport = viewport;
            self.events
                .push_back(BrowserEvent::ViewportChanged { view, viewport });
        }
        Ok(())
    }
    /// Reuse a frame allocation. Returns false and clears dimensions/pixels if
    /// the renderer has not produced a frame yet. Never pumps or blocks.
    pub fn read_frame(&mut self, view: ViewId, frame: &mut Frame) -> Result<bool, String> {
        self.view(view)?;
        (frame.width, frame.height) = self.engine.frame(view, &mut frame.pixels)?;
        if frame.width == 0 || frame.height == 0 {
            frame.pixels.clear();
            return Ok(false);
        }
        Ok(true)
    }
    pub fn mouse(&mut self, view: ViewId, input: MouseInput) -> Result<(), String> {
        self.view(view)?;
        self.engine.mouse(view, input);
        Ok(())
    }
    pub fn key(&mut self, view: ViewId, input: KeyInput<'_>) -> Result<(), String> {
        self.view(view)?;
        if input.name.contains('\0')
            || (input.codepoint != 0 && char::from_u32(input.codepoint).is_none())
        {
            return Err("invalid key name or Unicode scalar".into());
        }
        self.engine.key(
            view,
            input.pressed,
            input.name,
            input.codepoint,
            input.modifiers,
            input.repeat,
        );
        Ok(())
    }

    fn collect_events(&mut self) {
        for event in self.engine.events() {
            let view = event.view;
            let event = match event.kind {
                1 => {
                    self.views.insert(
                        view,
                        ViewState {
                            info: ViewInfo {
                                id: view,
                                url: String::new(),
                                title: String::new(),
                                viewport: self.default_viewport,
                                zoom: 1.,
                                metadata: ViewMetadata::default(),
                            },
                            load_script: String::new(),
                        },
                    );
                    if self.default_viewport != Viewport::default()
                        && let Err(message) = self.engine.resize(
                            view,
                            self.default_viewport.width,
                            self.default_viewport.height,
                            self.default_viewport.scale,
                        )
                    {
                        self.events.push_back(BrowserEvent::Error { message });
                    }
                    BrowserEvent::Opened { view }
                }
                2 => {
                    self.cancel_evaluations(view, "view closed");
                    self.views.remove(&view);
                    BrowserEvent::Closed { view }
                }
                3 => {
                    if let Some(state) = self.views.get_mut(&view) {
                        state.info.title.clone_from(&event.text);
                    }
                    BrowserEvent::Title {
                        view,
                        title: event.text,
                    }
                }
                4 => {
                    if let Some(state) = self.views.get_mut(&view) {
                        state.info.url.clone_from(&event.text);
                    }
                    BrowserEvent::Url {
                        view,
                        url: event.text,
                    }
                }
                5 => {
                    if let Some(state) = self.views.get(&view)
                        && !state.load_script.is_empty()
                        && let Err(message) = self.engine.action(view, 6, &state.load_script, 0.)
                    {
                        self.events.push_back(BrowserEvent::Error { message });
                    }
                    BrowserEvent::Loaded {
                        view,
                        url: event.text,
                    }
                }
                6 => BrowserEvent::FrameReady { view },
                7 => BrowserEvent::Hook {
                    view,
                    kind: "native".into(),
                    data: event.text,
                },
                8 => {
                    self.cancel_evaluations(view, "renderer failed");
                    BrowserEvent::Error {
                        message: event.text,
                    }
                }
                9 => match serde_json::from_str::<NativeEvaluation>(&event.text) {
                    Ok(reply) => {
                        if self
                            .evaluations
                            .get(&reply.request)
                            .is_none_or(|e| e.view != view)
                        {
                            continue;
                        }
                        self.evaluations.remove(&reply.request);
                        self.active_evaluations.remove(&view);
                        BrowserEvent::Evaluated {
                            view,
                            request: reply.request,
                            result: reply.result,
                        }
                    }
                    Err(error) => {
                        self.cancel_evaluations(view, "invalid native evaluation response");
                        BrowserEvent::Error {
                            message: error.to_string(),
                        }
                    }
                },
                10 => {
                    self.clipboard.clone_from(&event.text);
                    BrowserEvent::ClipboardChanged { text: event.text }
                }
                _ => continue,
            };
            self.events.push_back(event);
        }
    }
}
