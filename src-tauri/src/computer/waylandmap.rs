//! Wayland 後端的純邏輯（不碰 D-Bus，所以任何平台都能跑單元測試）：
//! PNG 解碼與縮放、物理像素↔邏輯座標、portal 的滾輪與按鈕代碼、`file://` 網址轉路徑。

use std::path::PathBuf;

/// 解 PNG 成 RGBA8。portal 截圖通常是 RGB 或 RGBA、8 位元；其他格式也先轉成 8 位元再補齊。
pub fn decode_png_rgba(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|e| format!("無法解析截圖 PNG：{e}"))?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("無法解碼截圖 PNG：{e}"))?;
    let (w, h) = (info.width, info.height);
    let data = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Indexed => return Err("不支援的截圖格式（索引色）".to_string()),
    };
    if rgba.len() != (w as usize) * (h as usize) * 4 {
        return Err("截圖像素資料大小與寬高不符".to_string());
    }
    Ok((w, h, rgba))
}

/// 以方塊平均縮小 RGBA（`tw`×`th` 不大於來源）。尺寸相同直接回傳副本。
pub fn downscale_rgba(src: &[u8], w: u32, h: u32, tw: u32, th: u32) -> Vec<u8> {
    if (w, h) == (tw, th) || tw == 0 || th == 0 || tw > w || th > h {
        return src.to_vec();
    }
    let mut out = Vec::with_capacity((tw as usize) * (th as usize) * 4);
    for ty in 0..th {
        let y0 = (ty as u64 * h as u64 / th as u64) as u32;
        let y1 = (((ty as u64 + 1) * h as u64 / th as u64) as u32).max(y0 + 1).min(h);
        for tx in 0..tw {
            let x0 = (tx as u64 * w as u64 / tw as u64) as u32;
            let x1 = (((tx as u64 + 1) * w as u64 / tw as u64) as u32).max(x0 + 1).min(w);
            let mut sum = [0u64; 4];
            let mut n = 0u64;
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y as usize) * (w as usize) + x as usize) * 4;
                    for c in 0..4 {
                        sum[c] += src[i + c] as u64;
                    }
                    n += 1;
                }
            }
            for s in sum {
                out.push((s / n) as u8);
            }
        }
    }
    out
}

pub fn rgba_to_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for p in out.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    out
}

/// 物理像素 ÷ 邏輯像素。縮放比例拿不到或不合理時當作 1.0。
pub fn scale_factor(image_w: u32, logical_w: i32) -> f64 {
    if logical_w <= 0 || image_w == 0 {
        return 1.0;
    }
    let s = image_w as f64 / logical_w as f64;
    if (0.5..=8.0).contains(&s) { s } else { 1.0 }
}

/// 螢幕邏輯座標 → portal 串流座標（相對於串流左上角，並夾在串流範圍內）。
pub fn stream_point(x: f64, y: f64, pos: (i32, i32), size: (i32, i32)) -> (f64, f64) {
    let max_x = (size.0 - 1).max(0) as f64;
    let max_y = (size.1 - 1).max(0) as f64;
    ((x - pos.0 as f64).clamp(0.0, max_x), (y - pos.1 as f64).clamp(0.0, max_y))
}

/// evdev 的按鈕代碼（portal 的 NotifyPointerButton 用這個）。
pub const BTN_LEFT: i32 = 0x110;
pub const BTN_RIGHT: i32 = 0x111;

/// 本程式的捲動量（`dy` 正＝往上、`dx` 正＝往右）→ portal 離散滾輪步數 `(水平, 垂直)`；
/// portal 的正值是往下／往右，所以垂直要反號。
pub fn scroll_steps(dx: i32, dy: i32) -> (i32, i32) {
    (dx.clamp(-200, 200), (-dy).clamp(-200, 200))
}

/// `file:///home/a/Pictures/Screenshot%20x.png` → 路徑。不是 `file://` 就回 `None`。
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    url::Url::parse(uri).ok()?.to_file_path().ok()
}

/// 具名按鍵 → Linux evdev keycode（`input-event-codes.h`）。portal 的 keysym 注入要由合成器反查鍵盤配置，
/// Enter 這類功能鍵在 GNOME 上實測會失效（文字有打進去、Enter 沒反應），所以功能鍵與修飾鍵改送實體 keycode。
/// 一般字元仍用 keysym（才不受鍵盤配置影響）。
pub fn evdev_keycode(key: &str) -> Option<i32> {
    Some(match key {
        "return" => 28,
        "tab" => 15,
        "space" => 57,
        "escape" => 1,
        "delete" => 14, // 我們的 delete ＝往回刪（BackSpace）
        "forwarddelete" => 111,
        "left" => 105,
        "up" => 103,
        "right" => 106,
        "down" => 108,
        "home" => 102,
        "end" => 107,
        "pageup" => 104,
        "pagedown" => 109,
        "f11" => 87,
        "f12" => 88,
        _ => match key.strip_prefix('f').and_then(|n| n.parse::<i32>().ok()) {
            Some(n) if (1..=10).contains(&n) => 58 + n, // F1=59 … F10=68
            _ => return None,
        },
    })
}

/// 修飾鍵的 evdev keycode（左 Ctrl 29、左 Shift 42、左 Alt 56）。`cmd` 在 Linux 當 Ctrl，與 `ctrl` 並用只按一次。
pub fn modifier_keycodes(cmd: bool, ctrl: bool, alt: bool, shift: bool) -> Vec<i32> {
    let mut v = Vec::new();
    if cmd || ctrl {
        v.push(29);
    }
    if alt {
        v.push(56);
    }
    if shift {
        v.push(42);
    }
    v
}

/// PipeWire 影格的像素排列（記憶體裡的位元組順序）。帶 alpha 的格式也一律當不透明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelLayout {
    /// B,G,R,x（也涵蓋 BGRA）
    Bgrx,
    /// R,G,B,x（也涵蓋 RGBA）
    Rgbx,
    /// x,R,G,B（也涵蓋 ARGB）
    Xrgb,
    /// x,B,G,R（也涵蓋 ABGR）
    Xbgr,
}

/// 一格影像 → RGBA8。`stride` 是每列的位元組數（可能大於 `width*4`）；資料不夠就回 `None`。
pub fn frame_to_rgba(layout: PixelLayout, width: u32, height: u32, stride: usize, data: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || stride < w * 4 || data.len() < stride * (h - 1) + w * 4 {
        return None;
    }
    let (r, g, b) = match layout {
        PixelLayout::Bgrx => (2, 1, 0),
        PixelLayout::Rgbx => (0, 1, 2),
        PixelLayout::Xrgb => (1, 2, 3),
        PixelLayout::Xbgr => (3, 2, 1),
    };
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = &data[y * stride..y * stride + w * 4];
        for px in row.chunks_exact(4) {
            out.extend_from_slice(&[px[r], px[g], px[b], 255]);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer::winmap::encode_png_rgba;

    #[test]
    fn png_round_trip_rgba() {
        let rgba: Vec<u8> = (0..4 * 3 * 4).map(|i| i as u8).collect();
        let png = encode_png_rgba(4, 3, &rgba).unwrap();
        let (w, h, back) = decode_png_rgba(&png).unwrap();
        assert_eq!((w, h), (4, 3));
        assert_eq!(back, rgba);
    }

    #[test]
    fn decodes_rgb_png_with_opaque_alpha() {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 2, 1);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[1, 2, 3, 4, 5, 6]).unwrap();
        }
        let (_, _, rgba) = decode_png_rgba(&out).unwrap();
        assert_eq!(rgba, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode_png_rgba(b"not a png").is_err());
    }

    #[test]
    fn downscale_averages_blocks() {
        // 2×2 → 1×1：四個像素的平均。
        let src = vec![0, 0, 0, 255, 100, 100, 100, 255, 100, 100, 100, 255, 200, 200, 200, 255];
        assert_eq!(downscale_rgba(&src, 2, 2, 1, 1), vec![100, 100, 100, 255]);
    }

    #[test]
    fn downscale_same_size_or_invalid_target_is_identity() {
        let src = vec![9u8; 2 * 2 * 4];
        assert_eq!(downscale_rgba(&src, 2, 2, 2, 2), src);
        assert_eq!(downscale_rgba(&src, 2, 2, 4, 4), src);
        assert_eq!(downscale_rgba(&src, 2, 2, 0, 1), src);
    }

    #[test]
    fn downscale_handles_fractional_ratio() {
        // 3×3 → 2×2（縮放 1.5）：輸出大小正確、不越界。
        let src = vec![50u8; 3 * 3 * 4];
        let out = downscale_rgba(&src, 3, 3, 2, 2);
        assert_eq!(out.len(), 2 * 2 * 4);
        assert!(out.iter().all(|v| *v == 50));
    }

    #[test]
    fn bgra_swap() {
        assert_eq!(rgba_to_bgra(&[1, 2, 3, 4]), vec![3, 2, 1, 4]);
    }

    #[test]
    fn scale_factor_is_sane() {
        assert_eq!(scale_factor(3840, 1920), 2.0);
        assert_eq!(scale_factor(1920, 1920), 1.0);
        assert_eq!(scale_factor(1920, 0), 1.0);
        assert_eq!(scale_factor(100, 5000), 1.0); // 離譜的值不採用
    }

    #[test]
    fn stream_point_offsets_and_clamps() {
        assert_eq!(stream_point(100.0, 50.0, (0, 0), (1920, 1080)), (100.0, 50.0));
        assert_eq!(stream_point(2000.0, 50.0, (0, 0), (1920, 1080)), (1919.0, 50.0));
        assert_eq!(stream_point(-5.0, -5.0, (0, 0), (1920, 1080)), (0.0, 0.0));
        assert_eq!(stream_point(1930.0, 10.0, (1920, 0), (1920, 1080)), (10.0, 10.0));
    }

    #[test]
    fn scroll_steps_flip_vertical() {
        assert_eq!(scroll_steps(0, 3), (0, -3)); // 往上
        assert_eq!(scroll_steps(2, -4), (2, 4)); // 往右、往下
        assert_eq!(scroll_steps(0, 9999), (0, -200));
    }

    // 只在 Unix 跑：Windows 上 Url::to_file_path 要求磁碟代號，`file:///home/…` 一律回 None。
    // 這個函式只給 Linux Wayland 後端用，Windows 上的結果不重要。
    #[cfg(unix)]
    #[test]
    fn file_uri_decodes_percent_escapes() {
        assert_eq!(
            file_uri_to_path("file:///home/a/Pictures/Screenshot%20x.png"),
            Some(PathBuf::from("/home/a/Pictures/Screenshot x.png"))
        );
        assert_eq!(file_uri_to_path("https://example.com/a.png"), None);
        assert_eq!(file_uri_to_path(""), None);
    }

    #[test]
    fn function_keys_map_to_evdev_codes() {
        assert_eq!(evdev_keycode("return"), Some(28));
        assert_eq!(evdev_keycode("delete"), Some(14));
        assert_eq!(evdev_keycode("forwarddelete"), Some(111));
        assert_eq!(evdev_keycode("f1"), Some(59));
        assert_eq!(evdev_keycode("f10"), Some(68));
        assert_eq!(evdev_keycode("f11"), Some(87));
        assert_eq!(evdev_keycode("f13"), None);
        assert_eq!(evdev_keycode("a"), None); // 字元走 keysym
    }

    #[test]
    fn modifiers_press_ctrl_once() {
        assert_eq!(modifier_keycodes(true, true, false, false), vec![29]);
        assert_eq!(modifier_keycodes(false, true, true, true), vec![29, 56, 42]);
        assert!(modifier_keycodes(false, false, false, false).is_empty());
    }

    #[test]
    fn frame_layouts_convert_to_rgba() {
        // 一個像素 R=10 G=20 B=30。
        assert_eq!(frame_to_rgba(PixelLayout::Bgrx, 1, 1, 4, &[30, 20, 10, 0]).unwrap(), vec![10, 20, 30, 255]);
        assert_eq!(frame_to_rgba(PixelLayout::Rgbx, 1, 1, 4, &[10, 20, 30, 7]).unwrap(), vec![10, 20, 30, 255]);
        assert_eq!(frame_to_rgba(PixelLayout::Xrgb, 1, 1, 4, &[0, 10, 20, 30]).unwrap(), vec![10, 20, 30, 255]);
        assert_eq!(frame_to_rgba(PixelLayout::Xbgr, 1, 1, 4, &[0, 30, 20, 10]).unwrap(), vec![10, 20, 30, 255]);
    }

    #[test]
    fn frame_stride_padding_is_skipped() {
        // 2×2，每列多 4 個位元組的填充（stride 12）。
        let data = [
            1, 2, 3, 0, 4, 5, 6, 0, 99, 99, 99, 99, //
            7, 8, 9, 0, 10, 11, 12, 0, 99, 99, 99, 99,
        ];
        let out = frame_to_rgba(PixelLayout::Rgbx, 2, 2, 12, &data).unwrap();
        assert_eq!(out, vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]);
        // 最後一列不需要填充也算夠。
        assert!(frame_to_rgba(PixelLayout::Rgbx, 2, 2, 12, &data[..20]).is_some());
    }

    #[test]
    fn short_or_bad_frames_are_rejected() {
        assert!(frame_to_rgba(PixelLayout::Rgbx, 2, 2, 8, &[0; 15]).is_none());
        assert!(frame_to_rgba(PixelLayout::Rgbx, 2, 2, 4, &[0; 64]).is_none()); // stride 比一列還短
        assert!(frame_to_rgba(PixelLayout::Rgbx, 0, 2, 8, &[0; 64]).is_none());
    }
}
