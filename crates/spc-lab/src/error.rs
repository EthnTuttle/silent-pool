//! Errors returned by the run engine.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    #[error("{code}: {detail}")]
    Failed { code: String, detail: String },
}

impl EngineError {
    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::Failed {
            code: code.into(),
            detail: detail.into(),
        }
    }
    pub fn code(&self) -> &str {
        match self {
            Self::Failed { code, .. } => code,
        }
    }
}
