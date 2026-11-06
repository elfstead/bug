use winit::{event_loop::OwnedDisplayHandle, raw_window_handle::HasDisplayHandle};

pub(super) enum Clipboard {
    Native(arboard::Clipboard),
    #[cfg(target_os = "linux")]
    Wayland {
        clipboard: smithay_clipboard::Clipboard,
        _display: OwnedDisplayHandle,
    },
}
impl Clipboard {
    pub fn new(display_owner: OwnedDisplayHandle) -> Result<Self, String> {
        let display = display_owner
            .display_handle()
            .map_err(|e| e.to_string())?
            .as_raw();
        #[cfg(target_os = "linux")]
        if let winit::raw_window_handle::RawDisplayHandle::Wayland(display) = display {
            // The owned display keeps its connection alive, and
            // the client explicitly drops this before destroying the loop.
            let clipboard = unsafe { smithay_clipboard::Clipboard::new(display.display.as_ptr()) };
            return Ok(Self::Wayland {
                clipboard,
                _display: display_owner,
            });
        }
        let _ = display;
        arboard::Clipboard::new()
            .map(Self::Native)
            .map_err(|e| e.to_string())
    }
    pub fn read(&mut self) -> Result<String, String> {
        match self {
            Self::Native(clipboard) => clipboard.get_text().map_err(|e| e.to_string()),
            #[cfg(target_os = "linux")]
            Self::Wayland { clipboard, .. } => clipboard.load().map_err(|e| e.to_string()),
        }
    }
    pub fn write(&mut self, text: &str) -> Result<(), String> {
        match self {
            Self::Native(clipboard) => clipboard.set_text(text).map_err(|e| e.to_string()),
            #[cfg(target_os = "linux")]
            Self::Wayland { clipboard, .. } => {
                clipboard.store(text);
                Ok(())
            }
        }
    }
}
