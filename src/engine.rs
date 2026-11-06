//! Single-threaded owner of the native bridge. No native pointer escapes this module.
use libloading::Library;
use std::{
    collections::VecDeque,
    ffi::{CString, c_char, c_void},
    marker::PhantomData,
    mem::ManuallyDrop,
    path::Path,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

// Ladybird has process-wide registries and TLS, so initialize it once per process.
static INITIALIZED: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
pub struct NativeEvent {
    pub kind: u32,
    pub view: u64,
    pub text: String,
}
type Callback = unsafe extern "C" fn(*mut c_void, u32, u64, *const c_char, usize);
type Init = unsafe extern "C" fn(*const c_char, *const c_char, Callback, *mut c_void) -> i32;
type Void = unsafe extern "C" fn();
type Open = unsafe extern "C" fn(*const c_char) -> u64;
type Navigate = unsafe extern "C" fn(u64, *const c_char) -> i32;
type Evaluate = unsafe extern "C" fn(u64, u64, *const c_char) -> i32;
type Action = unsafe extern "C" fn(u64, u32, *const c_char, f64) -> i32;
type Resize = unsafe extern "C" fn(u64, u32, u32, f64) -> i32;
type Frame = unsafe extern "C" fn(u64, *mut u32, usize, *mut u32, *mut u32) -> usize;
type Mouse = unsafe extern "C" fn(u64, u32, i32, i32, u32, u32, u32, f64, f64, u32) -> i32;
type Key = unsafe extern "C" fn(u64, i32, *const c_char, u32, u32, i32) -> i32;

pub struct Engine {
    // Ladybird owns process-wide state and event-loop TLS. Keep its code mapped
    // until process exit, including after native shutdown and failed startup.
    _library: ManuallyDrop<Library>,
    // C++ retains a pointer to the queue object, not just its allocation. Boxing
    // keeps that object stable when Engine is moved into the event-loop state.
    #[allow(clippy::box_collection)]
    events: Box<VecDeque<NativeEvent>>,
    shutdown: Void,
    pump: Void,
    open: Open,
    navigate: Navigate,
    evaluate: Evaluate,
    action: Action,
    resize: Resize,
    frame: Frame,
    mouse: Mouse,
    key: Key,
    _main_thread: PhantomData<Rc<()>>,
}

unsafe extern "C" fn receive(
    user: *mut c_void,
    kind: u32,
    view: u64,
    text: *const c_char,
    len: usize,
) {
    // Never unwind through C++. The bridge borrows these bytes only for this call.
    let result = std::panic::catch_unwind(|| {
        let bytes = if len == 0 {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(text.cast::<u8>(), len) }
        };
        let event = NativeEvent {
            kind,
            view,
            text: String::from_utf8_lossy(bytes).into_owned(),
        };
        unsafe { &mut *user.cast::<VecDeque<NativeEvent>>() }.push_back(event);
    });
    if result.is_err() {
        std::process::abort();
    }
}

impl Engine {
    pub fn load(library: &Path, executable: &Path, profile: &Path) -> Result<Self, String> {
        let executable = cstring(executable.to_str().ok_or("non-UTF-8 executable path")?)?;
        let profile = cstring(profile.to_str().ok_or("non-UTF-8 profile path")?)?;
        // SAFETY: The caller selects a trusted native library. Each symbol below
        // has the signature declared by native/bug.h, guarded by the ABI version.
        unsafe {
            let library = ManuallyDrop::new(
                Library::new(library).map_err(|e| format!("loading engine: {e}"))?,
            );
            let version = library
                .get::<unsafe extern "C" fn() -> u32>(b"bug_abi_version\0")
                .map_err(|e| e.to_string())?;
            if version() != 2 {
                return Err(
                    "unsupported Bug engine ABI (expected 2); rebuild with scripts/build-engine"
                        .into(),
                );
            }
            macro_rules! symbol {
                ($name:literal, $ty:ty) => {
                    *library
                        .get::<$ty>(concat!($name, "\0").as_bytes())
                        .map_err(|e| e.to_string())?
                };
            }
            let init = symbol!("bug_init", Init);
            if INITIALIZED.swap(true, Ordering::SeqCst) {
                return Err("Ladybird may only be initialized once per process".into());
            }
            let mut engine = Self {
                events: Box::default(),
                shutdown: symbol!("bug_shutdown", Void),
                pump: symbol!("bug_pump", Void),
                open: symbol!("bug_open", Open),
                navigate: symbol!("bug_navigate", Navigate),
                evaluate: symbol!("bug_evaluate", Evaluate),
                action: symbol!("bug_action", Action),
                resize: symbol!("bug_resize", Resize),
                frame: symbol!("bug_frame", Frame),
                mouse: symbol!("bug_mouse", Mouse),
                key: symbol!("bug_key", Key),
                _library: library,
                _main_thread: PhantomData,
            };
            let user = (&mut *engine.events as *mut VecDeque<NativeEvent>).cast();
            if init(executable.as_ptr(), profile.as_ptr(), receive, user) == 0 {
                return Err(engine
                    .events
                    .iter()
                    .map(|e| e.text.as_str())
                    .collect::<Vec<_>>()
                    .join("; "));
            }
            Ok(engine)
        }
    }
    pub fn poll(&mut self) {
        unsafe { (self.pump)() };
    }
    pub fn events(&mut self) -> VecDeque<NativeEvent> {
        std::mem::take(&mut *self.events)
    }
    pub fn open(&mut self, url: &str) -> Result<u64, String> {
        let url = cstring(url)?;
        let id = unsafe { (self.open)(url.as_ptr()) };
        if id == 0 {
            Err("could not create view".into())
        } else {
            Ok(id)
        }
    }
    pub fn navigate(&mut self, id: u64, url: &str) -> Result<(), String> {
        let url = cstring(url)?;
        success(unsafe { (self.navigate)(id, url.as_ptr()) }, id)
    }
    pub fn evaluate(&mut self, view: u64, request: u64, body: &str) -> Result<(), String> {
        let body = cstring(body)?;
        success(
            unsafe { (self.evaluate)(view, request, body.as_ptr()) },
            view,
        )
    }
    pub fn action(&mut self, id: u64, action: u32, text: &str, number: f64) -> Result<(), String> {
        let text = cstring(text)?;
        success(
            unsafe { (self.action)(id, action, text.as_ptr(), number) },
            id,
        )
    }
    pub fn resize(&mut self, id: u64, width: u32, height: u32, scale: f64) -> Result<(), String> {
        success(unsafe { (self.resize)(id, width, height, scale) }, id)
    }
    pub fn frame(&mut self, id: u64, pixels: &mut Vec<u32>) -> Result<(u32, u32), String> {
        let (mut width, mut height) = (0, 0);
        let count = unsafe { (self.frame)(id, std::ptr::null_mut(), 0, &mut width, &mut height) };
        if count == 0 {
            return Ok((0, 0));
        }
        if count != width as usize * height as usize || count > 64 * 1024 * 1024 {
            return Err("engine returned invalid frame dimensions".into());
        }
        pixels.resize(count, 0);
        let copied = unsafe {
            (self.frame)(
                id,
                pixels.as_mut_ptr(),
                pixels.len(),
                &mut width,
                &mut height,
            )
        };
        if copied != count {
            return Err("engine frame changed without an event-loop pump".into());
        }
        Ok((width, height))
    }
    pub fn mouse(&mut self, id: u64, input: MouseInput) {
        unsafe {
            (self.mouse)(
                id,
                input.kind,
                input.x,
                input.y,
                input.button,
                input.buttons,
                input.modifiers,
                input.dx,
                input.dy,
                input.clicks,
            );
        }
    }
    pub fn key(
        &mut self,
        id: u64,
        pressed: bool,
        name: &str,
        codepoint: u32,
        modifiers: u32,
        repeat: bool,
    ) {
        if let Ok(name) = cstring(name) {
            unsafe {
                (self.key)(
                    id,
                    pressed.into(),
                    name.as_ptr(),
                    codepoint,
                    modifiers,
                    repeat.into(),
                );
            }
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { (self.shutdown)() };
    }
}
#[derive(Default)]
pub struct MouseInput {
    pub kind: u32,
    pub x: i32,
    pub y: i32,
    pub button: u32,
    pub buttons: u32,
    pub modifiers: u32,
    pub dx: f64,
    pub dy: f64,
    pub clicks: u32,
}
fn cstring(text: &str) -> Result<CString, String> {
    CString::new(text).map_err(|_| "text contains a NUL byte".into())
}
fn success(value: i32, id: u64) -> Result<(), String> {
    if value != 0 {
        Ok(())
    } else {
        Err(format!("view {id}: missing view or unsupported argument"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_copies_borrowed_text_and_handles_empty_payloads() {
        let mut queue = VecDeque::<NativeEvent>::new();
        let user = (&mut queue as *mut VecDeque<NativeEvent>).cast();
        let mut text = b"hello".to_vec();
        unsafe {
            receive(user, 3, 9, text.as_ptr().cast(), text.len());
        }
        text.fill(b'x');
        unsafe {
            receive(user, 6, 9, std::ptr::null(), 0);
        }
        assert_eq!(queue[0].text, "hello");
        assert_eq!(queue[1].text, "");
    }
}
