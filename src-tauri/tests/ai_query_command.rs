//! End-to-end test for `ai_query` wiring. Uses a mock AiProvider so this
//! test is hermetic (no network, no real PTY).

use aiterm_lib::ai::{
    AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest,
};
use aiterm_lib::commands::ai::build_single_command_prompt;
use aiterm_lib::ai::context;
use aiterm_lib::pty::PtyManager;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

struct MockProvider {
    chunks: Vec<&'static str>,
}

#[async_trait]
impl AiProvider for MockProvider {
    fn id(&self) -> &str { "mock" }
    fn display_name(&self) -> &str { "Mock" }

    async fn generate(
        &self,
        _req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        for (i, c) in self.chunks.iter().enumerate() {
            let done = i + 1 == self.chunks.len();
            let _ = tx
                .send(GenerateChunk { delta: c.to_string(), done, usage: None })
                .await;
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), AiError> {
        Ok(())
    }
}

#[tokio::test]
async fn snapshot_builds_fallback_when_session_unknown() {
    let manager = PtyManager::new();
    let snap = context::snapshot(&manager, "no-such-session");
    // Fallback path: cwd is process cwd, shell is the default for this OS.
    assert!(!snap.cwd.as_os_str().is_empty());
    assert!(!snap.shell.is_empty());
}

#[test]
fn prompt_assembly_is_deterministic() {
    let snap = context::snapshot_from_parts("linux", "bash", PathBuf::from("/"));
    let a = build_single_command_prompt(&snap);
    let b = build_single_command_prompt(&snap);
    assert_eq!(a, b);
    assert!(a.contains("Shell: bash"));
}

// The full ai_query command requires a Tauri AppHandle + State, which cannot
// be constructed in unit tests. For M3 we exercise the pure parts (snapshot +
// prompt + mock provider protocol) — wiring is verified by the manual
// acceptance tests in Phase 6.

#[tokio::test]
async fn mock_provider_emits_chunks_through_channel() {
    let provider: Arc<dyn AiProvider> = Arc::new(MockProvider {
        chunks: vec![
            r#"{"explanation":"列出","command":"ls","#,
            r#""risk_level":"safe"}"#,
        ],
    });
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let req = GenerateRequest {
        system_prompt: "sys".into(),
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("list files"), tool_call_id: None, tool_calls: None }],
        context: context::snapshot_from_parts("linux", "bash", PathBuf::from("/")),
        mode: aiterm_lib::ai::QueryMode::SingleCommand,
        max_tokens: Some(256),
    };
    provider.generate(req, tx).await.expect("generate ok");

    let mut buf = String::new();
    while let Some(c) = rx.recv().await {
        buf.push_str(&c.delta);
        if c.done { break; }
    }
    let parsed: aiterm_lib::ai::AiSingleCommand = serde_json::from_str(&buf).expect("parse");
    assert_eq!(parsed.command, "ls");
    assert_eq!(parsed.explanation, "列出");
}

#[test]
fn prompt_includes_context_when_present() {
    let snap = aiterm_lib::ai::EnvSnapshot {
        os: "linux".into(),
        shell: "bash".into(),
        cwd: PathBuf::from("/home/user"),
        recent_output: Some("$ ls\nfoo  bar".into()),
        dir_listing: Some("bar\nfoo".into()),
    };
    let prompt = build_single_command_prompt(&snap);
    assert!(prompt.contains("Recent terminal output"));
    assert!(prompt.contains("Directory listing"));
    assert!(prompt.contains("foo  bar"));
}
