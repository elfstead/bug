//! Interactive actions shared by the CLI and the window client's palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Navigate,
    Open,
    Back,
    Forward,
    Reload,
    Stop,
    Close,
    ZoomIn,
    ZoomOut,
    ResetZoom,
    Quit,
    Eval,
    Views,
}

pub struct Description {
    pub action: Action,
    pub name: &'static str,
    pub label: &'static str,
    pub usage: &'static str,
    pub help: &'static str,
}
macro_rules! actions {
    ($(($action:ident, $name:literal, $label:literal, $usage:literal, $help:literal)),+ $(,)?) => {
        pub const ACTIONS: &[Description] = &[$(Description {
            action: Action::$action, name: $name, label: $label, usage: $usage, help: $help,
        }),+];
    };
}
actions![
    (
        Navigate,
        "navigate",
        "Navigate to URL",
        "[TARGET] URL",
        "Navigate an existing view."
    ),
    (
        Open,
        "open",
        "New window",
        "[--wait] URL",
        "Print the new view ID. --wait waits for its first loaded event."
    ),
    (
        Back,
        "back",
        "Back",
        "[TARGET]",
        "Go back in the view's history."
    ),
    (
        Forward,
        "forward",
        "Forward",
        "[TARGET]",
        "Go forward in the view's history."
    ),
    (Reload, "reload", "Reload", "[TARGET]", "Reload the page."),
    (
        Stop,
        "stop",
        "Stop loading",
        "[TARGET]",
        "Stop loading the page."
    ),
    (
        Close,
        "close",
        "Close window",
        "[TARGET]",
        "Request closure of this view."
    ),
    (
        ZoomIn,
        "zoom-in",
        "Zoom in",
        "[TARGET]",
        "Increase zoom by a factor of 1.2."
    ),
    (
        ZoomOut,
        "zoom-out",
        "Zoom out",
        "[TARGET]",
        "Decrease zoom by a factor of 1.2."
    ),
    (
        ResetZoom,
        "reset-zoom",
        "Reset zoom",
        "[TARGET]",
        "Restore the default zoom."
    ),
    (Quit, "quit", "Quit", "", "Shut down the running instance."),
    (
        Eval,
        "eval",
        "Evaluate JavaScript",
        "[TARGET] [--json] SCRIPT",
        "Await the script. Print strings as text and other values as JSON or JS scalars. --json preserves the tagged result. Exceptions exit 1."
    ),
    (
        Views,
        "views",
        "List views",
        "[--json]",
        "Print the current views as a JSON array, including focused and last_focused flags."
    ),
];

impl Action {
    pub fn takes_target(self) -> bool {
        !matches!(self, Action::Open | Action::Views | Action::Quit)
    }
    pub fn adjust_zoom(self, browser: &mut crate::Browser, view: u64) -> Result<(), String> {
        let current = browser.view(view)?.zoom;
        let zoom = match self {
            Action::ZoomIn => (current * 1.2).min(5.0),
            Action::ZoomOut => (current / 1.2).max(0.25),
            Action::ResetZoom => 1.0,
            _ => return Err("expected a zoom action".into()),
        };
        browser.set_zoom(view, zoom)
    }
}
