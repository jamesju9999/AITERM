//! 把 AITERM 的 AI 供應商接到 core 的 `AiComplete`（題目分類、群組「是不是新問題」用）。
use crate::ai::{AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest, QueryMode};
use aiterm_intake_core::host::AiComplete;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct ProviderAi(pub Arc<dyn AiProvider>);

/// 把一段文字連同系統提示丟給 AI，收完整段回覆。
async fn ask(provider: Arc<dyn AiProvider>, system_prompt: String, text: String) -> Result<String, AiError> {
    let req = GenerateRequest {
        system_prompt,
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::Value::String(text), tool_call_id: None, tool_calls: None }],
        context: Default::default(),
        mode: QueryMode::SingleCommand,
        max_tokens: None,
    };
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let p = provider.clone();
    let join = tokio::spawn(async move { p.generate(req, tx).await });
    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done {
            break;
        }
    }
    match join.await {
        Ok(Ok(())) => Ok(buf),
        Ok(Err(e)) => Err(e),
        Err(e) => Err(AiError::Network { message: e.to_string() }),
    }
}

#[async_trait::async_trait]
impl AiComplete for ProviderAi {
    async fn complete(&self, system: &str, user: &str) -> Result<String, String> {
        ask(self.0.clone(), system.to_string(), user.to_string()).await.map_err(|e| format!("{e:?}"))
    }
}

/// 每次呼叫才向 `AiRouter` 取目前設定的供應商；沒設定或失敗就回錯誤（呼叫端用各自的預設值）。
pub struct RouterAi<R: tauri::Runtime>(pub tauri::AppHandle<R>);

#[async_trait::async_trait]
impl<R: tauri::Runtime> AiComplete for RouterAi<R> {
    async fn complete(&self, system: &str, user: &str) -> Result<String, String> {
        use tauri::Manager;
        let router = self.0.try_state::<crate::ai::router::AiRouter>().ok_or("尚未設定 AI")?;
        let provider = router.resolve().await.map_err(|e| format!("{e:?}"))?;
        ProviderAi(provider).complete(system, user).await
    }
}
