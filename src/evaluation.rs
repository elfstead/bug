use serde::{Deserialize, Serialize};

pub type EvaluationId = u64;

/// A detached value, not a live reference to an object in the renderer. JSON
/// values are cloned; undefined, bigint, and non-finite numbers retain tags.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvaluationResult {
    Json {
        value: serde_json::Value,
    },
    Undefined,
    Bigint {
        value: String,
    },
    Number {
        value: String,
    },
    Error {
        name: String,
        message: String,
        #[serde(default)]
        stack: Option<String>,
    },
}
impl EvaluationResult {
    pub(crate) fn cancelled(message: &str) -> Self {
        Self::Error {
            name: "Cancelled".into(),
            message: message.into(),
            stack: None,
        }
    }
}

pub(crate) struct Evaluation {
    pub view: crate::ViewId,
    pub script: String,
}

#[derive(Deserialize)]
pub(crate) struct NativeEvaluation {
    pub request: EvaluationId,
    pub result: EvaluationResult,
}

pub(crate) fn function_body(script: &str) -> String {
    let quoted = serde_json::to_string(script).expect("serializing a string is infallible");
    format!(
        r#"return Promise.resolve().then(() => (0, eval)({quoted})).then(value => {{
        if (value === undefined) return {{type: 'undefined'}};
        if (typeof value === 'bigint') return {{type: 'bigint', value: String(value)}};
        if (typeof value === 'number' && !Number.isFinite(value)) return {{type: 'number', value: String(value)}};
        return {{type: 'json', value: JSON.parse(JSON.stringify(value))}};
    }}).catch(error => ({{type: 'error', name: String(error?.name ?? 'Error'),
        message: String(error?.message ?? error), stack: typeof error?.stack === 'string' ? error.stack : null}}));"#
    )
}
