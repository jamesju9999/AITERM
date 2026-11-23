//! 配對：用一次性邀請碼換長期 token。
//!
//! 邀請碼的祕密部分不上線。Client 與 server 各自對「這條 TLS 連線的 exporter」做 HMAC 互證：
//! 中間人終止 TLS 後面對的是兩條不同的連線（exporter 不同），又沒有邀請碼，所以既偽造不了
//! Client 的證明去換 token，也偽造不了 server 的證明去騙 Client。Client 驗過 server 的證明之後，
//! 才把這條連線上看到的憑證指紋釘住。
use crate::members;
use crate::state::AppState;
use crate::tls::{ConnExporter, PeerAddr};
use aiterm_core::share::auth;
use aiterm_intake_core::api::{ApiError, PairRequest, PairResponse, HEADER_CLIENT_PROOF, HEADER_HOST_PROOF};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use std::sync::Arc;
use std::time::Instant;

fn deny(status: StatusCode) -> Response {
    // 所有失敗原因（邀請不存在／過期／證明錯）回應相同，不洩漏哪一項。
    (status, Json(ApiError { error: "pairing failed".into() })).into_response()
}

pub async fn pair(
    State(st): State<Arc<AppState>>,
    Extension(exporter): Extension<ConnExporter>,
    Extension(peer): Extension<PeerAddr>,
    headers: HeaderMap,
    Json(req): Json<PairRequest>,
) -> Response {
    let ip = peer.0.ip();
    let now_i = Instant::now();
    if !st.backoff.delay_for(ip, now_i).is_zero() {
        return deny(StatusCode::TOO_MANY_REQUESTS);
    }
    let now = crate::db::now();
    let id = req.invite_id.trim().to_ascii_uppercase();
    let Some(invite) = members::find_invite(&st.pools.server, &id, now).await else {
        st.backoff.record_failure(ip, now_i);
        return deny(StatusCode::UNAUTHORIZED);
    };
    let proof = headers.get(HEADER_CLIENT_PROOF).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !auth::verify_viewer_proof(&invite.key, &exporter.0, proof) {
        st.backoff.record_failure(ip, now_i);
        members::record_invite_failure(&st.pools.server, &invite.id).await;
        return deny(StatusCode::UNAUTHORIZED);
    }
    let Ok(Some((me, token))) = members::redeem(&st.pools.server, &invite.id, now).await else {
        return deny(StatusCode::UNAUTHORIZED);
    };
    st.backoff.record_success(ip);
    crate::audit::record(&st.pools.server, &me.name, "pair", &me.id).await;
    let mut res = Json(PairResponse { token, me, fingerprint: st.fingerprint.clone() }).into_response();
    let host_proof = auth::host_proof(&invite.key, &exporter.0);
    if let Ok(v) = HeaderValue::from_str(&host_proof) {
        res.headers_mut().insert(HEADER_HOST_PROOF, v);
    }
    res
}
