//! Requests that an external controller can take over for a view.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    FilePicker,
    Alert,
    Confirm,
    Prompt,
    BeforeUnload,
    Permission,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequestDetails {
    FilePicker {
        multiple: bool,
        accept: Vec<String>,
    },
    Alert {
        message: String,
    },
    Confirm {
        message: String,
    },
    Prompt {
        message: String,
        default: String,
    },
    BeforeUnload {
        source: String,
    },
    Permission {
        name: String,
        origin: String,
        is_file: bool,
        system_denied: bool,
        show_os_hint: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserRequest {
    pub request: u64,
    #[serde(flatten)]
    pub details: RequestDetails,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAnswer {
    Dismiss,
    AllowOnce,
    AllowAlways,
    Block,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequestReply {
    FilePicker { paths: Vec<PathBuf> },
    Alert,
    Confirm { accept: bool },
    Prompt { text: Option<String> },
    BeforeUnload { leave: bool },
    Permission { decision: PermissionAnswer },
}
