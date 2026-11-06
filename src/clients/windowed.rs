//! winit/softbuffer is one client of the public Browser API.
use super::{
    Controller, RunOptions, Session,
    palette::{Palette, Result as PaletteResult},
};
use crate::{Browser, BrowserEvent, Frame, KeyInput, MouseInput, ViewId, Viewport};
use softbuffer::{Context, Surface};
use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU32,
    rc::Rc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, ModifiersState, NamedKey, PhysicalKey},
    window::{Window, WindowId},
};

struct Page {
    view: ViewId,
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    _context: Context<Rc<Window>>,
    frame: Frame,
    cursor: (i32, i32),
    buttons: u32,
    modifiers: ModifiersState,
    last_click: Option<(Instant, (i32, i32), u32)>,
    clicks: u32,
    palette: Option<Palette>,
    suppressed_keys: HashSet<PhysicalKey>,
    suppressed_mouse: HashSet<MouseButton>,
}
struct App<C> {
    clipboard: Option<super::clipboard::Clipboard>,
    pages: HashMap<WindowId, Page>,
    session: Session<C>,
    started: bool,
}

pub fn run(
    browser: Browser,
    controller: impl Controller,
    options: RunOptions,
) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let mut app = App {
        clipboard: None,
        pages: HashMap::new(),
        session: Session::new(browser, controller, options)?,
        started: false,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.session.result()
}

impl<C: Controller> App<C> {
    fn make_window(&mut self, event_loop: &ActiveEventLoop, view: ViewId) -> Result<(), String> {
        let viewport = self.session.browser.view(view)?.viewport;
        let window = Rc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Bug")
                        .with_decorations(false)
                        .with_inner_size(PhysicalSize::new(viewport.width, viewport.height)),
                )
                .map_err(|e| e.to_string())?,
        );
        let context = Context::new(window.clone()).map_err(|e| e.to_string())?;
        if self.clipboard.is_none() {
            match super::clipboard::Clipboard::new(event_loop.owned_display_handle()) {
                Ok(clipboard) => self.clipboard = Some(clipboard),
                Err(error) => self.session.controller.event(&BrowserEvent::Hook {
                    view,
                    kind: "clipboard_unavailable".into(),
                    data: error,
                }),
            }
        }
        let surface = Surface::new(&context, window.clone()).map_err(|e| e.to_string())?;
        let size = window.inner_size();
        self.session.browser.resize(
            view,
            Viewport {
                width: size.width.max(1),
                height: size.height.max(1),
                scale: window.scale_factor(),
            },
        )?;
        self.pages.insert(
            window.id(),
            Page {
                view,
                window,
                surface,
                _context: context,
                frame: Frame::default(),
                cursor: (0, 0),
                buttons: 0,
                modifiers: ModifiersState::empty(),
                last_click: None,
                clicks: 0,
                palette: None,
                suppressed_keys: HashSet::new(),
                suppressed_mouse: HashSet::new(),
            },
        );
        Ok(())
    }
    fn handle_events(&mut self, event_loop: &ActiveEventLoop, events: Vec<BrowserEvent>) {
        for event in events {
            match event {
                BrowserEvent::ClipboardChanged { text } => {
                    if let Some(clipboard) = &mut self.clipboard
                        && let Err(message) = clipboard.write(&text)
                    {
                        self.session.controller.event(&BrowserEvent::Error {
                            message: format!("clipboard: {message}"),
                        });
                    }
                }
                BrowserEvent::Opened { view } => {
                    // A controller may open and close a view in the same turn.
                    if self.session.browser.view(view).is_err() {
                        continue;
                    }
                    if let Err(e) = self.make_window(event_loop, view) {
                        self.session.report_error(e);
                        self.session.quit = true;
                    }
                }
                BrowserEvent::Closed { view } => self.pages.retain(|_, page| page.view != view),
                BrowserEvent::Title { view, title } => {
                    if let Some(page) = self.pages.values_mut().find(|p| p.view == view) {
                        page.window.set_title(&title);
                    }
                }
                BrowserEvent::FrameReady { view } => {
                    if let Some(page) = self.pages.values().find(|p| p.view == view) {
                        page.window.request_redraw();
                    }
                }
                BrowserEvent::FocusRequested { view } => {
                    if let Some(page) = self.pages.values().find(|p| p.view == view) {
                        page.window.focus_window();
                    }
                }
                BrowserEvent::ViewportChanged { view, viewport } => {
                    if let Some(page) = self.pages.values().find(|p| p.view == view) {
                        let size = PhysicalSize::new(viewport.width, viewport.height);
                        if page.window.inner_size() != size {
                            let _ = page.window.request_inner_size(size);
                        }
                    }
                }
                _ => {}
            }
        }
        if self.session.quit {
            event_loop.exit();
        }
    }
    fn draw(&mut self, id: WindowId) -> Result<(), String> {
        let Some(page) = self.pages.get_mut(&id) else {
            return Ok(());
        };
        self.session
            .browser
            .read_frame(page.view, &mut page.frame)?;
        let size = page.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return Ok(());
        };
        page.surface.resize(w, h).map_err(|e| e.to_string())?;
        let mut buffer = page.surface.buffer_mut().map_err(|e| e.to_string())?;
        buffer.fill(0x00ffffff);
        for y in 0..page.frame.height.min(size.height) as usize {
            let n = page.frame.width.min(size.width) as usize;
            buffer[y * size.width as usize..y * size.width as usize + n].copy_from_slice(
                &page.frame.pixels
                    [y * page.frame.width as usize..y * page.frame.width as usize + n],
            );
        }
        if let Some(palette) = &page.palette {
            palette.draw(
                &mut buffer,
                size.width,
                size.height,
                page.window.scale_factor(),
            );
        }
        page.window.pre_present_notify();
        buffer.present().map_err(|e| e.to_string())
    }
    fn palette_result(&mut self, id: WindowId, result: Result<PaletteResult, String>) {
        let Some(page) = self.pages.get_mut(&id) else {
            return;
        };
        match result {
            Ok(PaletteResult::Keep) => {}
            Ok(PaletteResult::Close) => {
                page.palette = None;
                let _ = self
                    .session
                    .browser
                    .set_focused(page.view, page.window.has_focus());
                self.session.controller.event(&BrowserEvent::Hook {
                    view: page.view,
                    kind: "palette".into(),
                    data: "closed".into(),
                });
            }
            Ok(PaletteResult::Quit) => self.session.quit = true,
            Err(message) => self
                .session
                .controller
                .event(&BrowserEvent::Error { message }),
        }
        page.window.request_redraw();
    }
    fn keyboard(&mut self, id: WindowId, event: KeyEvent) -> Result<(), String> {
        let Some(page) = self.pages.get_mut(&id) else {
            return Ok(());
        };
        let pressed = event.state == ElementState::Pressed;
        if !pressed && page.suppressed_keys.remove(&event.physical_key) {
            return Ok(());
        }
        let shortcut = super::shortcuts::palette(
            &event.logical_key,
            page.modifiers,
            cfg!(target_os = "macos"),
        );
        if pressed && shortcut {
            page.suppressed_keys.insert(event.physical_key);
            if event.repeat {
                return Ok(());
            }
            if page.palette.is_some() {
                self.palette_result(id, Ok(PaletteResult::Close));
            } else {
                page.palette = Some(Palette::default());
                page.buttons = 0;
                self.session.browser.set_focused(page.view, false)?;
                self.session.controller.event(&BrowserEvent::Hook {
                    view: page.view,
                    kind: "palette".into(),
                    data: "opened".into(),
                });
                page.window.request_redraw();
            }
            return Ok(());
        }
        let clipboard_key = if pressed {
            super::shortcuts::clipboard(
                &event.logical_key,
                page.modifiers,
                cfg!(target_os = "macos"),
            )
        } else {
            None
        };
        if let Some(key) = clipboard_key {
            page.suppressed_keys.insert(event.physical_key);
            if key == "v" {
                let text = self
                    .clipboard
                    .as_mut()
                    .ok_or("system clipboard unavailable")?
                    .read()?;
                if let Some(palette) = &mut page.palette {
                    palette.text(&text);
                    page.window.request_redraw();
                } else {
                    self.session.browser.paste(page.view, &text)?;
                }
            } else if let Some(palette) = &mut page.palette {
                self.clipboard
                    .as_mut()
                    .ok_or("system clipboard unavailable")?
                    .write(palette.query())?;
                if key == "x" {
                    palette.clear();
                    page.window.request_redraw();
                }
            } else if key == "c" {
                self.session.browser.copy(page.view)?;
            } else {
                self.session.browser.cut(page.view)?;
            }
            return Ok(());
        }
        if let Some(palette) = &mut page.palette {
            if pressed {
                page.suppressed_keys.insert(event.physical_key);
                let result = match &event.logical_key {
                    Key::Named(NamedKey::Escape) => Ok(palette.escape()),
                    Key::Named(NamedKey::Enter) => {
                        palette.submit(&mut self.session.browser, page.view)
                    }
                    Key::Named(NamedKey::ArrowUp) => {
                        palette.select(-1);
                        Ok(PaletteResult::Keep)
                    }
                    Key::Named(NamedKey::ArrowDown) => {
                        palette.select(1);
                        Ok(PaletteResult::Keep)
                    }
                    Key::Named(NamedKey::Backspace) => {
                        palette.backspace();
                        Ok(PaletteResult::Keep)
                    }
                    Key::Character(s)
                        if page.modifiers.control_key() && s.eq_ignore_ascii_case("u") =>
                    {
                        palette.clear();
                        Ok(PaletteResult::Keep)
                    }
                    _ => {
                        if !page.modifiers.control_key()
                            && !page.modifiers.alt_key()
                            && !page.modifiers.super_key()
                            && let Some(text) = &event.text
                        {
                            palette.text(text);
                        }
                        Ok(PaletteResult::Keep)
                    }
                };
                self.palette_result(id, result);
            }
            return Ok(());
        }
        self.session.browser.key(
            page.view,
            KeyInput {
                pressed,
                name: &key_name(&event.logical_key),
                codepoint: event
                    .text
                    .as_ref()
                    .and_then(|s| s.chars().next())
                    .map_or(0, |c| c as u32),
                modifiers: modifiers(page.modifiers),
                repeat: event.repeat,
            },
        )
    }
    fn input(&mut self, id: WindowId, event: WindowEvent) -> Result<(), String> {
        if let WindowEvent::KeyboardInput { event, .. } = event {
            return self.keyboard(id, event);
        }
        let Some(page) = self.pages.get_mut(&id) else {
            return Ok(());
        };
        let view = page.view;
        match event {
            WindowEvent::CloseRequested => self.session.browser.close(view)?,
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                self.session.browser.resize(
                    view,
                    Viewport {
                        width: size.width,
                        height: size.height,
                        scale: page.window.scale_factor(),
                    },
                )?;
                page.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let size = page.window.inner_size();
                self.session.browser.resize(
                    view,
                    Viewport {
                        width: size.width.max(1),
                        height: size.height.max(1),
                        scale: scale_factor,
                    },
                )?;
                page.window.request_redraw();
            }
            WindowEvent::Focused(focused) => {
                self.session
                    .browser
                    .set_focused(view, focused && page.palette.is_none())?;
                if !focused {
                    page.buttons = 0;
                    page.modifiers = ModifiersState::empty();
                }
            }
            WindowEvent::ModifiersChanged(m) => page.modifiers = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                page.cursor = (position.x as i32, position.y as i32);
                if page.palette.is_none() {
                    self.session.browser.mouse(view, mouse(page, 0))?;
                }
            }
            WindowEvent::CursorLeft { .. } if page.palette.is_none() => {
                self.session.browser.mouse(view, mouse(page, 4))?
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if state == ElementState::Released && page.suppressed_mouse.remove(&button) {
                    return Ok(());
                }
                if let Some(palette) = &mut page.palette {
                    if state == ElementState::Pressed {
                        page.suppressed_mouse.insert(button);
                    }
                    let size = page.window.inner_size();
                    if button == MouseButton::Left
                        && state == ElementState::Pressed
                        && palette.click(
                            page.cursor.0,
                            page.cursor.1,
                            size.width,
                            size.height,
                            page.window.scale_factor(),
                        )
                    {
                        let result = palette.submit(&mut self.session.browser, view);
                        self.palette_result(id, result);
                    }
                    return Ok(());
                }
                let button = match button {
                    MouseButton::Left => 1,
                    MouseButton::Right => 2,
                    MouseButton::Middle => 4,
                    MouseButton::Back => 8,
                    MouseButton::Forward => 16,
                    _ => 0,
                };
                let pressed = state == ElementState::Pressed;
                if pressed {
                    page.buttons |= button;
                    let now = Instant::now();
                    let repeated = page.last_click.is_some_and(|(t, p, b)| {
                        now.duration_since(t) < Duration::from_millis(400)
                            && (p.0 - page.cursor.0).abs() < 5
                            && (p.1 - page.cursor.1).abs() < 5
                            && b == button
                    });
                    page.clicks = if repeated { page.clicks % 3 + 1 } else { 1 };
                    page.last_click = Some((now, page.cursor, button));
                } else {
                    page.buttons &= !button;
                }
                let mut input = mouse(page, if pressed { 1 } else { 2 });
                input.button = button;
                self.session.browser.mouse(view, input)?;
            }
            WindowEvent::MouseWheel { delta, .. } if page.palette.is_none() => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-(x as f64) * 40., -(y as f64) * 40.),
                    MouseScrollDelta::PixelDelta(p) => (-p.x, -p.y),
                };
                let mut input = mouse(page, 3);
                input.dx = dx;
                input.dy = dy;
                self.session.browser.mouse(view, input)?;
            }
            _ => {}
        }
        Ok(())
    }
}
impl<C: Controller> ApplicationHandler for App<C> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.started {
            return;
        }
        self.started = true;
        if let Err(e) = self.session.start() {
            self.session.report_error(e);
            self.session.quit = true;
        }
        let events = self.session.events();
        self.handle_events(event_loop, events);
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let result = if matches!(event, WindowEvent::RedrawRequested) {
            self.draw(id)
        } else {
            self.input(id, event)
        };
        if let Err(e) = result {
            self.session.report_error(e);
        }
        let events = self.session.events();
        self.handle_events(event_loop, events);
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let events = self.session.step();
        self.handle_events(event_loop, events);
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(8),
        ));
    }
    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.clipboard = None;
        self.pages.clear();
    }
}
fn modifiers(m: ModifiersState) -> u32 {
    u32::from(m.alt_key())
        | (u32::from(m.control_key()) << 1)
        | (u32::from(m.shift_key()) << 2)
        | (u32::from(m.super_key()) << 3)
}
fn mouse(page: &Page, kind: u32) -> MouseInput {
    MouseInput {
        kind,
        x: page.cursor.0,
        y: page.cursor.1,
        buttons: page.buttons,
        modifiers: modifiers(page.modifiers),
        clicks: page.clicks,
        ..Default::default()
    }
}
fn key_name(key: &Key) -> String {
    match key {
        Key::Character(s) => s.to_uppercase(),
        Key::Named(n) => match n {
            NamedKey::Enter => "Return",
            NamedKey::ArrowLeft => "Left",
            NamedKey::ArrowRight => "Right",
            NamedKey::ArrowUp => "Up",
            NamedKey::ArrowDown => "Down",
            NamedKey::Shift => "LeftShift",
            NamedKey::Control => "LeftControl",
            NamedKey::Alt => "LeftAlt",
            NamedKey::Super => "LeftGUI",
            NamedKey::Space => "Space",
            _ => return format!("{n:?}"),
        }
        .into(),
        _ => "Invalid".into(),
    }
}
