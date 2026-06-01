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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_node_roundtrip() {
        let node = DocNode {
            title: "Getting Started".into(),
            href: "/docs/getting-started".into(),
            items: vec![DocNode {
                title: "Quickstart".into(),
                href: "/docs/quickstart".into(),
                items: vec![],
            }],
        };
        let json = serde_json::to_string(&node).unwrap();
        let decoded: DocNode = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.title, "Getting Started");
        assert_eq!(decoded.items[0].href, "/docs/quickstart");
    }

    #[test]
    fn keep_options_defaults() {
        let opts: KeepOptions = serde_json::from_str("{}").unwrap();
        assert!(opts.description);
        assert!(opts.parameters);
        assert!(opts.request_body);
        assert!(opts.responses);
        assert!(opts.code_samples);
    }

    #[test]
    fn extraction_options_roundtrip() {
        let opts = ExtractionOptions {
            url: "https://docs.example.com".into(),
            pages: vec!["/api/v1".into()],
            output_dir: "/tmp/out".into(),
            merge: true,
            keep: KeepOptions::default(),
            cookies: "session=abc".into(),
        };
        let json = serde_json::to_string(&opts).unwrap();
        let decoded: ExtractionOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.url, "https://docs.example.com");
        assert!(decoded.merge);
        assert_eq!(decoded.cookies, "session=abc");
    }
}
