// Generated from native/abi.toml by scripts/generate-abi. Edit the definition instead.
use std::ffi::{c_char, c_int, c_void};

pub(super) const ABI_VERSION: u32 = 3;

pub(super) type BugEvent = unsafe extern "C" fn(*mut c_void, u32, u64, *const c_char, usize);

pub(crate) mod events {
    pub(crate) const OPENED: u32 = 1;
    pub(crate) const CLOSED: u32 = 2;
    pub(crate) const TITLE: u32 = 3;
    pub(crate) const URL: u32 = 4;
    pub(crate) const LOADED: u32 = 5;
    pub(crate) const FRAME: u32 = 6;
    pub(crate) const HOOK: u32 = 7;
    pub(crate) const ERROR: u32 = 8;
    pub(crate) const EVALUATED: u32 = 9;
    pub(crate) const CLIPBOARD: u32 = 10;
    pub(crate) const FILE_PICKER: u32 = 11;
}

pub(crate) mod actions {
    pub(crate) const BACK: u32 = 1;
    pub(crate) const FORWARD: u32 = 2;
    pub(crate) const RELOAD: u32 = 3;
    pub(crate) const STOP: u32 = 4;
    pub(crate) const CLOSE: u32 = 5;
    pub(crate) const EVAL: u32 = 6;
    pub(crate) const ZOOM: u32 = 7;
    pub(crate) const FOCUS: u32 = 8;
    pub(crate) const BLUR: u32 = 9;
    pub(crate) const COPY: u32 = 10;
    pub(crate) const CUT: u32 = 11;
    pub(crate) const PASTE: u32 = 12;
}

pub(super) struct Symbols {
    pub(super) init: unsafe extern "C" fn(*const c_char, *const c_char, BugEvent, *mut c_void) -> c_int,
    pub(super) shutdown: unsafe extern "C" fn(),
    pub(super) pump: unsafe extern "C" fn(),
    pub(super) open: unsafe extern "C" fn(*const c_char) -> u64,
    pub(super) navigate: unsafe extern "C" fn(u64, *const c_char) -> c_int,
    pub(super) evaluate: unsafe extern "C" fn(u64, u64, *const c_char) -> c_int,
    pub(super) reply_file_picker: unsafe extern "C" fn(u64, u64, *const *const c_char, usize) -> c_int,
    pub(super) action: unsafe extern "C" fn(u64, u32, *const c_char, f64) -> c_int,
    pub(super) resize: unsafe extern "C" fn(u64, u32, u32, f64) -> c_int,
    pub(super) frame: unsafe extern "C" fn(u64, *mut u32, usize, *mut u32, *mut u32) -> usize,
    pub(super) mouse: unsafe extern "C" fn(u64, u32, i32, i32, u32, u32, u32, f64, f64, u32) -> c_int,
    pub(super) key: unsafe extern "C" fn(u64, c_int, *const c_char, u32, u32, c_int) -> c_int,
}

impl Symbols {
    pub(super) unsafe fn load(library: &libloading::Library) -> Result<Self, String> {
        let version: unsafe extern "C" fn() -> u32 =
            unsafe { super::load_symbol(library, c"bug_abi_version")? };
        let found = unsafe { version() };
        if found != ABI_VERSION {
            return Err(format!(
                "unsupported Bug engine ABI {found} (expected {ABI_VERSION}); rebuild with scripts/build-engine"
            ));
        }
        unsafe {
            Ok(Self {
                init: super::load_symbol(library, c"bug_init")?,
                shutdown: super::load_symbol(library, c"bug_shutdown")?,
                pump: super::load_symbol(library, c"bug_pump")?,
                open: super::load_symbol(library, c"bug_open")?,
                navigate: super::load_symbol(library, c"bug_navigate")?,
                evaluate: super::load_symbol(library, c"bug_evaluate")?,
                reply_file_picker: super::load_symbol(library, c"bug_reply_file_picker")?,
                action: super::load_symbol(library, c"bug_action")?,
                resize: super::load_symbol(library, c"bug_resize")?,
                frame: super::load_symbol(library, c"bug_frame")?,
                mouse: super::load_symbol(library, c"bug_mouse")?,
                key: super::load_symbol(library, c"bug_key")?,
            })
        }
    }
}
