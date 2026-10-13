//! 沒開 `wayland-pipewire` 功能時（AppImage：在 Ubuntu 22.04 建置，PipeWire 太舊）的替身。
//! 介面與 `pipewire_capture` 相同；`start` 一律失敗，呼叫端就退回截圖 portal。

use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Capture;

impl Capture {
    pub fn start(_fd: OwnedFd, _node: u32) -> Result<Capture, String> {
        Err("這個版本沒有內建 PipeWire 串流取畫面（AppImage），改用截圖 portal".to_string())
    }

    pub async fn frame(&self, _since: Instant, _wait: Duration) -> Option<Frame> {
        None
    }
}
