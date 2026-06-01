// src-tauri/src/api_docs/types.rs
use serde::{Deserialize, Serialize};

/// A node in the documentation tree (mirrors Python DocNode)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocNode {
    pub title: String,
    pub href: String,
    #[serde(default)]
    pub items: Vec<DocNode>,
}

/// Which parts of each endpoint to include in the Markdown output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeepOptions {
    #[serde(default = "default_true")]
    pub description: bool,
    #[serde(default = "default_true")]
    pub parameters: bool,
    #[serde(default = "default_true")]
    pub request_body: bool,
    #[serde(default = "default_true")]
    pub responses: bool,
    #[serde(default = "default_true")]
    pub code_samples: bool,
}

fn default_true() -> bool { true }

impl Default for KeepOptions {
    fn default() -> Self {
        Self {
            description: true,
            parameters: true,
            request_body: true,
            responses: true,
            code_samples: true,
        }
    }
}

/// Options passed to the `extract` subcommand
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionOptions {
    pub url: String,
    /// Pages selected from the tree (href values)
    pub pages: Vec<String>,
    pub output_dir: String,
    /// true = single merged file, false = one file per page
    pub merge: bool,
    pub keep: KeepOptions,
    /// Serialised cookie string "k=v; k2=v2" (may be empty)
    #[serde(default)]
    pub cookies: String,
}

/// Response from api_docs_auth_status
#[derive(Debug, Serialize)]
pub struct AuthStatus {
    pub logged_in: bool,
    /// Account name / email if known, otherwise empty
    pub account: String,
}
