use std::collections::BTreeMap;

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppError {
    pub code: String,
    pub params: BTreeMap<String, String>,
}

impl AppError {
    pub fn new(code: &str) -> Self {
        Self {
            code: code.into(),
            params: BTreeMap::new(),
        }
    }

    pub fn with(mut self, k: &str, v: impl Into<String>) -> Self {
        self.params.insert(k.into(), v.into());
        self
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code)?;
        if !self.params.is_empty() {
            let joined = self
                .params
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(", ");
            write!(f, " ({joined})")?;
        }
        Ok(())
    }
}

impl std::error::Error for AppError {}
