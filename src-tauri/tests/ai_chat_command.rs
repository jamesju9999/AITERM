//! Integration test for `ai_chat`. Uses a `MockProvider` so this test is
//! hermetic (no network, no real PTY).

use aiterm_lib::ai::{
    AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest, Locale, QueryMode,
};
use aiterm_lib::commands::ai::build_chat_prompt;
use aiterm_lib::ai::context;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// MockProvider that records the last `GenerateRequest` it received and
/// emits a fixed sequence of chunks.
#[derive(Clone)]
struct MockProvider {
    chunks: Vec<&'static str>,
    last_request: Arc<Mutex<Option<CapturedRequest>>>,
}

#[derive(Clone, Debug)]
struct CapturedRequest {
    messages: Vec<ChatMessage>,
    system_prompt: String,
    mode: QueryMode,
    max_tokens: Option<u32>,
}

impl MockProvider {
    fn new(chunks: Vec<&'static str>) -> Self {
        Self {
            chunks,
            last_request: Arc::new(Mutex::new(None)),
        }
    }
}

#[async_trait]
impl AiProvider for MockProvider {
    fn id(&self) -> &str { "mock" }
    fn display_name(&self) -> &str { "Mock" }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        *self.last_request.lock().unwrap() = Some(CapturedRequest {
            messages: req.messages.clone(),
            system_prompt: req.system_prompt.clone(),
            mode: req.mode,
            max_tokens: req.max_tokens,
        });
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

/// Direct call to the inner chat-generate loop without the Tauri State wiring.
/// This exercises the same code path the `ai_chat` command runs, but can be
/// called from a plain #[tokio::test] without a full AppHandle.
///
/// (We duplicate the loop here because `ai_chat` is a `#[tauri::command]` that
/// can only be invoked via the Tauri runtime. The copied logic is small and
/// must stay in sync with ai.rs — that's why we also test the prompt builder
/// separately in the lib test.)
///
/// NOT exercised by this helper (intentional):
///   1. The input validation guards in `ai_chat` (empty history rejection and
///      "last message must be from user" rejection). Those live outside the
///      generate loop and require a Tauri runtime harness to hit.
///   2. `app.emit("ai-stream", ...)` per-chunk event emission. The helper
///      buffers chunks directly. A regression that stops emitting events —
///      or stops emitting `done: true` — will NOT be caught here.
async fn run_chat_loop(
    provider: Arc<dyn AiProvider>,
    messages: Vec<ChatMessage>,
    locale: Locale,
) -> Result<String, AiError> {
    let snapshot = context::snapshot_from_parts(
        "linux",
        "bash",
        std::path::PathBuf::from("/"),
    );
    let prompt = build_chat_prompt(&snapshot, locale, false);
    let req = GenerateRequest {
        system_prompt: prompt,
        messages,
        context: snapshot,
        mode: QueryMode::Chat,
        max_tokens: Some(1024),
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_for_spawn = provider.clone();
    let join = tokio::spawn(async move { provider_for_spawn.generate(req, tx).await });

    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }

    match join.await {
        Ok(Ok(())) => Ok(buf),
        Ok(Err(e)) => Err(e),
        Err(join_err) => Err(AiError::Network { message: join_err.to_string() }),
    }
}

fn user(text: &str) -> ChatMessage {
    ChatMessage { role: "user".into(), content: serde_json::json!(text), tool_call_id: None, tool_calls: None }
}

fn assistant(text: &str) -> ChatMessage {
    ChatMessage { role: "assistant".into(), content: serde_json::json!(text), tool_call_id: None, tool_calls: None }
}

#[tokio::test]
async fn chat_returns_raw_content_without_json_parsing() {
    let mock = MockProvider::new(vec![
        "看你要做的事情，建議執行 ",
        "<cmd>ls -la</cmd>",
        " 試試看。",
    ]);
    let provider: Arc<dyn AiProvider> = Arc::new(mock.clone());

    let content = run_chat_loop(provider, vec![user("列出所有檔案")], Locale::ZhTw)
        .await
        .expect("chat should succeed");

    // Raw content, including <cmd> tag, must come back verbatim.
    assert!(content.contains("<cmd>ls -la</cmd>"));
    assert!(content.starts_with("看你要做"));
}

#[tokio::test]
async fn chat_passes_full_message_history_to_provider() {
    let mock = MockProvider::new(vec!["ok"]);
    let captured = mock.last_request.clone();
    let provider: Arc<dyn AiProvider> = Arc::new(mock);

    let history = vec![
        user("第一輪問題"),
        assistant("第一輪回答"),
        user("第二輪問題"),
        assistant("第二輪回答"),
        user("第三輪問題"),
    ];
    run_chat_loop(provider, history.clone(), Locale::ZhTw).await.expect("ok");

    let got = captured.lock().unwrap().clone().expect("captured request");
    assert_eq!(got.messages.len(), 5, "all 5 messages must be forwarded");
    assert_eq!(got.messages[0].content, "第一輪問題");
    assert_eq!(got.messages[4].content, "第三輪問題");
    assert!(matches!(got.mode, QueryMode::Chat));
    assert_eq!(got.max_tokens, Some(1024));
}

#[tokio::test]
async fn chat_prompt_is_chat_not_single_command() {
    let mock = MockProvider::new(vec!["x"]);
    let captured = mock.last_request.clone();
    let provider: Arc<dyn AiProvider> = Arc::new(mock);

    run_chat_loop(provider, vec![user("hi")], Locale::ZhTw).await.expect("ok");

    let got = captured.lock().unwrap().clone().unwrap();
    assert!(got.system_prompt.contains("<cmd>"), "chat prompt must mention <cmd>");
    assert!(
        !got.system_prompt.contains("Output ONLY a JSON object"),
        "chat prompt must not be the single-command prompt"
    );
}

#[tokio::test]
async fn chat_propagates_provider_network_error() {
    struct FailingProvider;
    #[async_trait]
    impl AiProvider for FailingProvider {
        fn id(&self) -> &str { "fail" }
        fn display_name(&self) -> &str { "Fail" }
        async fn generate(
            &self,
            _req: GenerateRequest,
            _tx: mpsc::Sender<GenerateChunk>,
        ) -> Result<(), AiError> {
            Err(AiError::Network { message: "boom".into() })
        }
        async fn health_check(&self) -> Result<(), AiError> { Ok(()) }
    }

    let provider: Arc<dyn AiProvider> = Arc::new(FailingProvider);
    let err = run_chat_loop(provider, vec![user("hi")], Locale::ZhTw).await.unwrap_err();
    match err {
        AiError::Network { message } => assert_eq!(message, "boom"),
        other => panic!("expected Network, got {other:?}"),
    }
}

#[tokio::test]
async fn chat_prompt_language_rule_follows_locale() {
    let mock = MockProvider::new(vec!["x"]);
    let captured = mock.last_request.clone();
    let provider: Arc<dyn AiProvider> = Arc::new(mock);

    run_chat_loop(provider, vec![user("hi")], Locale::En).await.expect("ok");

    let got = captured.lock().unwrap().clone().unwrap();
    assert!(got.system_prompt.contains("Respond in English."), "prompt: {}", got.system_prompt);
}
