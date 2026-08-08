//! 把上游回應接成送給 Claude Code 的 SSE byte 串流。
//!
//! 開場的 ping：Claude Code 在等上游第一個 byte 期間遇到 SSE 靜默會斷線，
//! 而 SSE 註解（`: ping`）不算資料。所以在等待上游回應時每 3 秒送一個真的
//! `event: ping` frame。本地模型冷啟動可能要好幾十秒，這條保護是必要的。

use std::time::Duration;

use axum::body::Bytes;
use futures_util::StreamExt;
use serde_json::Value;

use crate::bridge::anthropic::request::MessagesRequest;
use crate::bridge::anthropic::response::{ping_frame, SseEncoder};
use crate::bridge::factory::{build, Upstream};
use crate::bridge::server::{error_frame_for, AppState};
use crate::bridge::upstream::anthropic::ClientHeaders;
use crate::bridge::upstream::UpstreamResponse;
use crate::config::types::TierMapping;

/// 每隔多久補一個 ping。3 秒遠低於任何合理的客戶端逾時，成本可忽略。
const PING_INTERVAL: Duration = Duration::from_secs(3);

pub fn run(
    state: AppState,
    mapping: TierMapping,
    req: MessagesRequest,
    raw: Value,
    message_id: String,
    client_headers: ClientHeaders,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send {
    let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(64);

    tokio::spawn(async move {
        let send = |b: Bytes| {
            let tx = tx.clone();
            async move { tx.send(b).await.is_ok() }
        };

        // ── 解析上游憑證並發出請求，期間持續送 ping ──────────────────
        let upstream_fut = async {
            let up = build(&state.config, &state.secrets, &state.tool_meta, &mapping.provider_id).await?;
            match up {
                Upstream::Anthropic(a) => a.send_raw(&raw, &mapping.model, &client_headers).await,
                Upstream::OpenAi(o) => {
                    use crate::bridge::upstream::BridgeUpstream;
                    o.send(&req, &mapping.model).await
                }
                Upstream::Codex(c) => {
                    use crate::bridge::upstream::BridgeUpstream;
                    c.send(&req, &mapping.model).await
                }
                Upstream::Antigravity(a) => {
                    use crate::bridge::upstream::BridgeUpstream;
                    a.send(&req, &mapping.model).await
                }
            }
        };
        tokio::pin!(upstream_fut);

        // 必須用 interval_at 而非 interval：後者的第一個 tick 期限是
        // Instant::now()，也就是「已經到期」。在下面的 select! 裡它會贏過
        // 上游請求（上游至少要一次 async hop），於是每一個請求都會在
        // message_start 之前多送一個 ping，而不是只有慢的上游才送。
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + PING_INTERVAL,
            PING_INTERVAL,
        );
        let resp = loop {
            tokio::select! {
                r = &mut upstream_fut => break r,
                _ = ticker.tick() => {
                    if !send(Bytes::from(ping_frame())).await { return; }
                }
            }
        };

        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                log::warn!("bridge upstream 失敗 provider={} err={e:?}", mapping.provider_id);
                let _ = send(Bytes::from(error_frame_for(&e))).await;
                return;
            }
        };

        match resp {
            // Anthropic 家族：原樣 pipe，不解析也不重組。
            UpstreamResponse::Passthrough(r) => {
                let mut bytes = r.bytes_stream();
                while let Some(chunk) = bytes.next().await {
                    match chunk {
                        Ok(b) => {
                            if !send(b).await { return; }
                        }
                        Err(e) => {
                            log::warn!("bridge passthrough 串流中斷：{e}");
                            return;
                        }
                    }
                }
            }
            UpstreamResponse::Events(mut events) => {
                // input_tokens 用估算值：多數 OpenAI 相容端點要到串流結束才
                // 給 usage，但 Claude Code 在 message_start 就要讀它來算
                // context 與自動壓縮門檻。任何合理的估算都好過保證錯的 0。
                let mut enc = SseEncoder::new(
                    message_id,
                    req.model.clone(),
                    crate::bridge::server::estimate_input_tokens(&req),
                );
                for f in enc.start() {
                    if !send(Bytes::from(f)).await { return; }
                }
                while let Some(item) = events.next().await {
                    match item {
                        Ok(ev) => {
                            for f in enc.push(ev) {
                                if !send(Bytes::from(f)).await { return; }
                            }
                        }
                        Err(e) => {
                            log::warn!("bridge 串流中斷 provider={} err={e:?}", mapping.provider_id);
                            let _ = send(Bytes::from(error_frame_for(&e))).await;
                            return;
                        }
                    }
                }
                // 上游沒送 Done 就結束時補收尾，否則 Claude Code 會永遠等下去。
                // 已收過 Done 時 finish() 回空 vec，可安全無條件呼叫。
                for f in enc.finish() {
                    if !send(Bytes::from(f)).await { return; }
                }
            }
        }
    });

    futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|b| (Ok(b), rx))
    })
}
