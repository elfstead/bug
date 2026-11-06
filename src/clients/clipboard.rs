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
    // arboard uses NSPasteboard on macOS and X11 on Linux. Only Wayland needs
    // the compositor connection owned by the windowing client.
    fn native() -> Result<Self, String> {
        arboard::Clipboard::new()
            .map(Self::Native)
            .map_err(|e| e.to_string())
    }

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
        Self::native()
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::Clipboard;
    use std::{
        io::Write,
        process::{Command, Stdio},
    };

    // Run explicitly on an ephemeral CI runner: this overwrites its clipboard.
    #[test]
    #[ignore = "requires the macOS pasteboard service and overwrites the system clipboard"]
    fn macos_system_clipboard_interoperates_with_pbcopy_and_pbpaste() {
        let mut clipboard = Clipboard::native().expect("NSPasteboard is available");
        let from_system = "macOS → Bug: 日本語 🐛\nsecond line";
        let mut writer = Command::new("/usr/bin/pbcopy")
            .env("LC_ALL", "en_US.UTF-8")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        writer
            .stdin
            .take()
            .unwrap()
            .write_all(from_system.as_bytes())
            .unwrap();
        assert!(writer.wait().unwrap().success());
        assert_eq!(clipboard.read().unwrap(), from_system);

        let from_bug = "Bug → macOS: café 🐛\nsecond line";
        clipboard.write(from_bug).unwrap();
        drop(clipboard); // macOS retains the pasteboard contents after the writer is dropped.
        let reader = Command::new("/usr/bin/pbpaste")
            .env("LC_ALL", "en_US.UTF-8")
            .output()
            .unwrap();
        assert!(reader.status.success());
        assert_eq!(String::from_utf8(reader.stdout).unwrap(), from_bug);
    }
}
