use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "status")]
pub enum CommandOutput {
    #[serde(rename = "success")]
    Success { data: serde_json::Value },
    #[serde(rename = "error")]
    Error {
        error: String,
        context: Option<String>,
    },
}

impl CommandOutput {
    pub fn success(data: serde_json::Value) -> Self {
        Self::Success { data }
    }

    #[allow(dead_code)]
    pub fn error(error: String, context: Option<String>) -> Self {
        Self::Error { error, context }
    }
}
