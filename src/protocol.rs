//! The external scripting protocol. Engine calls themselves stay in-process.
use crate::{
    Browser, BrowserEvent, EvaluationResult, Frame, SessionSnapshot, ViewMetadata, Viewport,
    clients::Control,
};
use serde::{Deserialize, Serialize};
use std::io::Read;

/// One JSON object per line on stdin. IDs are stable for the lifetime of a view.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    /// Select unsolicited notifications for the JSON stream. Null restores all.
    Subscribe {
        events: Option<Vec<Notification>>,
    },
    Key {
        view: u64,
        pressed: bool,
        name: String,
        #[serde(default)]
        codepoint: u32,
        #[serde(default)]
        modifiers: u32,
        #[serde(default)]
        repeat: bool,
    },
    Mouse {
        view: u64,
        kind: MouseKind,
        x: i32,
        y: i32,
        #[serde(default)]
        button: u32,
        #[serde(default)]
        buttons: u32,
        #[serde(default)]
        modifiers: u32,
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
        #[serde(default)]
        clicks: u32,
    },
    SetRequestHandlers {
        view: u64,
        kinds: Vec<crate::RequestKind>,
    },
    ReplyRequest {
        view: u64,
        request: u64,
        reply: crate::RequestReply,
    },
    SetDocumentStartScript {
        view: u64,
        script: String,
    },
    Click {
        view: u64,
        x: i32,
        y: i32,
    },
    ReplyFilePicker {
        view: u64,
        request: u64,
        paths: Vec<std::path::PathBuf>,
    },
    SetMetadata {
        view: u64,
        metadata: ViewMetadata,
    },
    Find {
        name: String,
    },
    Snapshot {},
    Restore {
        session: SessionSnapshot,
    },
    SaveSession {
        path: String,
    },
    LoadSession {
        path: String,
    },
    Copy {
        view: u64,
    },
    Cut {
        view: u64,
    },
    Paste {
        view: u64,
        text: String,
    },
    Open {
        url: String,
        #[serde(default)]
        document_start_script: String,
    },
    Navigate {
        view: u64,
        url: String,
    },
    Back {
        view: u64,
    },
    Forward {
        view: u64,
    },
    Reload {
        view: u64,
    },
    Stop {
        view: u64,
    },
    Close {
        view: u64,
    },
    Focus {
        view: u64,
    },
    Eval {
        view: u64,
        script: String,
    },
    ZoomIn {
        view: u64,
    },
    ZoomOut {
        view: u64,
    },
    ResetZoom {
        view: u64,
    },
    Zoom {
        view: u64,
        factor: f64,
    },
    /// Inject a script after each subsequent top-level load in this view.
    SetLoadScript {
        view: u64,
        script: String,
    },
    Resize {
        view: u64,
        width: u32,
        height: u32,
        #[serde(default = "default_scale")]
        scale: f64,
    },
    Screenshot {
        view: u64,
        path: String,
    },
    List {},
    Quit {},
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[repr(u32)]
pub enum MouseKind {
    Move = 0,
    Down = 1,
    Up = 2,
    Wheel = 3,
    Leave = 4,
}

fn default_scale() -> f64 {
    1.0
}

impl Command {
    pub fn parse(line: &str) -> Result<Self, String> {
        let command: Self = serde_json::from_str(line).map_err(|e| e.to_string())?;
        command.validate()?;
        Ok(command)
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Open { url, .. } | Self::Navigate { url, .. } if url.contains('\0') => {
                return Err("URL contains a NUL byte".into());
            }
            Self::Zoom { factor, .. } if !factor.is_finite() || !(0.25..=5.0).contains(factor) => {
                return Err("zoom factor must be between 0.25 and 5".into());
            }
            _ => {}
        }
        Ok(())
    }

    /// Adapt a wire command to the library API. No window system is involved.
    pub fn execute(self, browser: &mut Browser) -> Result<(Control, Option<Event>), String> {
        match self {
            Self::Subscribe { events } => {
                return Ok((Control::Continue, Some(Event::Subscribed { events })));
            }
            Self::SetMetadata { view, metadata } => browser.set_metadata(view, metadata)?,
            Self::Find { name } => {
                return Ok((
                    Control::Continue,
                    Some(Event::Found {
                        name: name.clone(),
                        view: browser.find_view(&name),
                    }),
                ));
            }
            Self::Snapshot {} => {
                return Ok((
                    Control::Continue,
                    Some(Event::Session {
                        session: browser.snapshot(),
                    }),
                ));
            }
            Self::Restore { session } => {
                return Ok((
                    Control::Continue,
                    Some(Event::Restored {
                        views: browser.restore(&session)?,
                    }),
                ));
            }
            Self::SaveSession { path } => {
                std::fs::write(
                    &path,
                    serde_json::to_vec_pretty(&browser.snapshot()).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                return Ok((Control::Continue, Some(Event::SessionSaved { path })));
            }
            Self::LoadSession { path } => {
                return Ok((
                    Control::Continue,
                    Some(Event::Restored {
                        views: browser.restore(&read_session(&path)?)?,
                    }),
                ));
            }
            Self::Key {
                view,
                pressed,
                name,
                codepoint,
                modifiers,
                repeat,
            } => browser.key(
                view,
                crate::KeyInput {
                    pressed,
                    name: &name,
                    codepoint,
                    modifiers,
                    repeat,
                },
            )?,
            Self::Mouse {
                view,
                kind,
                x,
                y,
                button,
                buttons,
                modifiers,
                dx,
                dy,
                clicks,
            } => browser.mouse(
                view,
                crate::MouseInput {
                    kind: kind as u32,
                    x,
                    y,
                    button,
                    buttons,
                    modifiers,
                    dx,
                    dy,
                    clicks,
                },
            )?,
            Self::SetRequestHandlers { view, kinds } => {
                browser.set_request_handlers(view, &kinds)?
            }
            Self::ReplyRequest {
                view,
                request,
                reply,
            } => {
                if !browser.reply_request(view, request, reply)? {
                    return Err("request is no longer pending".into());
                }
            }
            Self::SetDocumentStartScript { view, script } => {
                browser.set_document_start_script(view, &script)?
            }
            Self::Click { view, x, y } => browser.click(view, x, y)?,
            Self::ReplyFilePicker {
                view,
                request,
                paths,
            } => {
                if !browser.reply_file_picker(view, request, &paths)? {
                    return Err("file picker request is no longer pending".into());
                }
            }
            Self::Copy { view } => browser.copy(view)?,
            Self::Cut { view } => browser.cut(view)?,
            Self::Paste { view, text } => browser.paste(view, &text)?,
            Self::Open {
                url,
                document_start_script,
            } => {
                let view = browser.open_with_document_start_script(&url, &document_start_script)?;
                return Ok((Control::Continue, Some(Event::Opened { view })));
            }
            Self::Navigate { view, url } => browser.navigate(view, &url)?,
            Self::Back { view } => browser.back(view)?,
            Self::Forward { view } => browser.forward(view)?,
            Self::Reload { view } => browser.reload(view)?,
            Self::Stop { view } => browser.stop(view)?,
            Self::Close { view } => browser.close(view)?,
            Self::Focus { view } => browser.request_focus(view)?,
            Self::Eval { view, script } => {
                return Ok((
                    Control::Continue,
                    Some(Event::EvaluationQueued {
                        view,
                        request: browser.eval(view, &script)?,
                    }),
                ));
            }
            Self::ZoomIn { view } => crate::commands::Action::ZoomIn.adjust_zoom(browser, view)?,
            Self::ZoomOut { view } => {
                crate::commands::Action::ZoomOut.adjust_zoom(browser, view)?
            }
            Self::ResetZoom { view } => {
                crate::commands::Action::ResetZoom.adjust_zoom(browser, view)?
            }
            Self::Zoom { view, factor } => browser.set_zoom(view, factor)?,
            Self::SetLoadScript { view, script } => browser.set_load_script(view, &script)?,
            Self::Resize {
                view,
                width,
                height,
                scale,
            } => browser.resize(
                view,
                Viewport {
                    width,
                    height,
                    scale,
                },
            )?,
            Self::Screenshot { view, path } => {
                let mut frame = Frame::default();
                if !browser.read_frame(view, &mut frame)? {
                    return Err("view has not produced a frame yet".into());
                }
                frame.write_ppm(&path)?;
                return Ok((
                    Control::Continue,
                    Some(Event::Screenshot {
                        view,
                        path,
                        width: frame.width,
                        height: frame.height,
                    }),
                ));
            }
            Self::List {} => {
                return Ok((
                    Control::Continue,
                    Some(Event::Views {
                        views: browser
                            .views()
                            .map(|v| ViewInfo {
                                view: v.id,
                                focused: browser.focused_view() == Some(v.id),
                                last_focused: browser.last_focused_view() == Some(v.id),
                                url: v.url.clone(),
                                title: v.title.clone(),
                                viewport: v.viewport,
                                zoom: v.zoom,
                                metadata: v.metadata.clone(),
                            })
                            .collect(),
                    }),
                ));
            }
            Self::Quit {} => return Ok((Control::Quit, None)),
        }
        Ok((Control::Continue, None))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ViewInfo {
    pub focused: bool,
    pub last_focused: bool,
    pub view: u64,
    pub url: String,
    pub title: String,
    pub viewport: Viewport,
    pub zoom: f64,
    pub metadata: ViewMetadata,
}

#[derive(Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Subscribed {
        events: Option<Vec<Notification>>,
    },
    Request {
        view: u64,
        #[serde(flatten)]
        callback: crate::BrowserRequest,
    },
    RequestCancelled {
        view: u64,
        request: u64,
    },
    FilePickerRequested {
        view: u64,
        #[serde(flatten)]
        picker: crate::FilePickerRequest,
    },
    EvaluationQueued {
        view: u64,
        request: u64,
    },
    Evaluated {
        view: u64,
        request: u64,
        result: EvaluationResult,
    },
    Metadata {
        view: u64,
        metadata: ViewMetadata,
    },
    Found {
        name: String,
        view: Option<u64>,
    },
    Session {
        session: SessionSnapshot,
    },
    Restored {
        views: Vec<u64>,
    },
    SessionSaved {
        path: String,
    },
    Clipboard {
        text: String,
    },
    Resized {
        view: u64,
        width: u32,
        height: u32,
        scale: f64,
    },
    Ready {
        protocol: u32,
    },
    Opened {
        view: u64,
    },
    Closed {
        view: u64,
    },
    Focused {
        view: u64,
    },
    Url {
        view: u64,
        url: String,
    },
    Title {
        view: u64,
        title: String,
    },
    Loaded {
        view: u64,
        url: String,
    },
    Hook {
        view: u64,
        kind: String,
        data: String,
    },
    Views {
        views: Vec<ViewInfo>,
    },
    Error {
        message: String,
    },
    Screenshot {
        view: u64,
        path: String,
        width: u32,
        height: u32,
    },
}

impl Event {
    pub fn from_browser(event: &BrowserEvent) -> Option<Self> {
        Some(match event {
            BrowserEvent::Request { view, callback } => Self::Request {
                view: *view,
                callback: callback.clone(),
            },
            BrowserEvent::RequestCancelled { view, request } => Self::RequestCancelled {
                view: *view,
                request: *request,
            },
            BrowserEvent::FilePickerRequested { view, picker } => Self::FilePickerRequested {
                view: *view,
                picker: picker.clone(),
            },
            BrowserEvent::Evaluated {
                view,
                request,
                result,
            } => Self::Evaluated {
                view: *view,
                request: *request,
                result: result.clone(),
            },
            BrowserEvent::MetadataChanged { view, metadata } => Self::Metadata {
                view: *view,
                metadata: metadata.clone(),
            },
            BrowserEvent::ClipboardChanged { text } => Self::Clipboard { text: text.clone() },
            BrowserEvent::ViewportChanged { view, viewport } => Self::Resized {
                view: *view,
                width: viewport.width,
                height: viewport.height,
                scale: viewport.scale,
            },
            BrowserEvent::Opened { view } => Self::Opened { view: *view },
            BrowserEvent::Closed { view } => Self::Closed { view: *view },
            BrowserEvent::Title { view, title } => Self::Title {
                view: *view,
                title: title.clone(),
            },
            BrowserEvent::Url { view, url } => Self::Url {
                view: *view,
                url: url.clone(),
            },
            BrowserEvent::Loaded { view, url } => Self::Loaded {
                view: *view,
                url: url.clone(),
            },
            BrowserEvent::Focused { view } => Self::Focused { view: *view },
            BrowserEvent::Hook { view, kind, data } => Self::Hook {
                view: *view,
                kind: kind.clone(),
                data: data.clone(),
            },
            BrowserEvent::Error { message } => Self::Error {
                message: message.clone(),
            },
            BrowserEvent::FrameReady { .. } | BrowserEvent::FocusRequested { .. } => return None,
        })
    }
}

/// Notifications that can be filtered independently of replies and pending requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notification {
    Opened,
    Closed,
    Focused,
    Url,
    Title,
    Loaded,
    Metadata,
    Clipboard,
    Resized,
    Hook,
}
impl Event {
    fn notification(&self) -> Option<Notification> {
        Some(match self {
            Self::Opened { .. } => Notification::Opened,
            Self::Closed { .. } => Notification::Closed,
            Self::Focused { .. } => Notification::Focused,
            Self::Url { .. } => Notification::Url,
            Self::Title { .. } => Notification::Title,
            Self::Loaded { .. } => Notification::Loaded,
            Self::Metadata { .. } => Notification::Metadata,
            Self::Clipboard { .. } => Notification::Clipboard,
            Self::Resized { .. } => Notification::Resized,
            Self::Hook { .. } => Notification::Hook,
            _ => return None,
        })
    }
}

mod controller;
mod output;
pub use controller::JsonController;

/// Read a versioned session document with a bounded allocation.
pub fn read_session(path: impl AsRef<std::path::Path>) -> Result<SessionSnapshot, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("session exceeds 16 MiB".into());
    }
    let session: SessionSnapshot = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    session.validate()?;
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_script_and_url_as_data() {
        let script = "document.title = '\"; $(touch /tmp/no); 日本語\\n';";
        let json = serde_json::json!({"cmd":"eval", "view":42, "script":script});
        assert_eq!(
            Command::parse(&json.to_string()).unwrap(),
            Command::Eval {
                view: 42,
                script: script.into()
            }
        );
    }

    #[test]
    fn rejects_bad_commands_before_ffi() {
        for line in [
            r#"{"cmd":"zoom","view":1,"factor":0}"#,
            r#"{"cmd":"navigate","view":-1,"url":"x"}"#,
            r#"{"cmd":"quit","surprise":true}"#,
            r#"{"cmd":"open","url":"x\u0000y"}"#,
        ] {
            assert!(Command::parse(line).is_err(), "{line}");
        }
    }

    #[test]
    fn events_escape_newlines_into_one_protocol_record() {
        let event = Event::Title {
            view: 7,
            title: "a\nb\"c".into(),
        };
        let line = serde_json::to_string(&event).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line).unwrap()["title"],
            "a\nb\"c"
        );
    }
}
