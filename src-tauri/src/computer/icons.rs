//! 找「只有圖形、沒有文字也沒有無障礙資訊」的按鈕（例如 LINE 聊天視窗標頭的電話圖示）。
//!
//! 做法：在視窗的某個區域（上／下／左／右）裡，找和背景對比明顯的小塊、近似正方形的連通區域。
//! 純影像運算、與平台無關，所以可以用合成圖完整測試。它只回答「哪裡有圖示、由左到右第幾個」，
//! 不知道圖示是什麼意思——用途要由呼叫端（提示詞裡的版面知識、動作後的畫面回報）判斷。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Top,
    Bottom,
    Left,
    Right,
    All,
}

impl Region {
    pub fn parse(s: &str) -> Result<Region, String> {
        match s.trim().to_lowercase().as_str() {
            "top" | "header" => Ok(Region::Top),
            "bottom" | "footer" => Ok(Region::Bottom),
            "left" => Ok(Region::Left),
            "right" => Ok(Region::Right),
            "all" => Ok(Region::All),
            other => Err(format!("region 只能是 top、bottom、left、right 或 all，收到「{other}」")),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Region::Top => "上方",
            Region::Bottom => "下方",
            Region::Left => "左側",
            Region::Right => "右側",
            Region::All => "整個視窗",
        }
    }

    /// 區域在整張圖裡的像素範圍 (x0, y0, x1, y1)。上下取 15%，左右取 20%。
    fn bounds(&self, w: usize, h: usize) -> (usize, usize, usize, usize) {
        match self {
            Region::Top => (0, 0, w, (h as f64 * 0.15).ceil() as usize),
            Region::Bottom => (0, (h as f64 * 0.85) as usize, w, h),
            Region::Left => (0, 0, (w as f64 * 0.2).ceil() as usize, h),
            Region::Right => ((w as f64 * 0.8) as usize, 0, w, h),
            Region::All => (0, 0, w, h),
        }
    }
}

/// 像素座標的矩形（整張圖的座標系）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// 圖示的大小範圍（邏輯像素）。比這小的是雜訊，比這大的是頭像／圖片，太扁或太瘦的是文字或分隔線。
const MIN_SIDE: f64 = 9.0;
const MAX_SIDE: f64 = 34.0;
const MIN_ASPECT: f64 = 0.55;
const MAX_ASPECT: f64 = 1.8;
/// 選單點點（「⋮」「···」）：細的那邊在 MIN_THIN~MAX_THIN 之間（太粗就是文字或圖片），長寬比不超過這個值。
const MIN_THIN: f64 = 2.5;
const MAX_THIN: f64 = 6.0;
const MAX_THIN_RATIO: f64 = 7.0;
/// 與背景亮度差超過這個值才算前景。
const CONTRAST: i32 = 45;

fn median(values: &mut [u8]) -> u8 {
    values.sort_unstable();
    values[values.len() / 2]
}

/// `lum`：每個像素 0~255 的亮度，列優先；`scale`：像素數 / 邏輯像素（Retina 為 2）。
/// 回傳區域內的圖示候選，由左到右（同一水平帶內）、由上到下排序。
pub fn detect_icons(lum: &[u8], width: usize, height: usize, scale: f64, region: Region) -> Vec<Rect> {
    if width == 0 || height == 0 || lum.len() < width * height {
        return vec![];
    }
    let (x0, y0, x1, y1) = region.bounds(width, height);
    let (cw, ch) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    if cw < 4 || ch < 4 {
        return vec![];
    }

    // 背景亮度：區域內的中位數（深色或淺色主題都適用）。
    let mut sample: Vec<u8> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).step_by(3).map(|(x, y)| lum[y * width + x]).collect();
    let bg = median(&mut sample) as i32;

    let mut mask = vec![false; cw * ch];
    for y in 0..ch {
        for x in 0..cw {
            mask[y * cw + x] = (lum[(y0 + y) * width + (x0 + x)] as i32 - bg).abs() > CONTRAST;
        }
    }

    // 膨脹：把圖示的細線條（電話聽筒、放大鏡）連成一塊；半徑約 1.5 邏輯像素。
    let r = (1.5 * scale).round().max(1.0) as usize;
    let dilated = dilate(&mask, cw, ch, r);

    // 連通區域（4 連通）。
    let mut seen = vec![false; cw * ch];
    let mut found: Vec<Rect> = Vec::new();
    let mut stack = Vec::new();
    for sy in 0..ch {
        for sx in 0..cw {
            let i = sy * cw + sx;
            if !dilated[i] || seen[i] {
                continue;
            }
            let (mut minx, mut miny, mut maxx, mut maxy) = (sx, sy, sx, sy);
            let mut pixels = 0usize;
            stack.push((sx, sy));
            seen[i] = true;
            while let Some((x, y)) = stack.pop() {
                pixels += 1;
                minx = minx.min(x);
                maxx = maxx.max(x);
                miny = miny.min(y);
                maxy = maxy.max(y);
                for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
                    if nx < cw && ny < ch {
                        let j = ny * cw + nx;
                        if dilated[j] && !seen[j] {
                            seen[j] = true;
                            stack.push((nx, ny));
                        }
                    }
                }
            }
            let _ = pixels;
            // 去掉膨脹加上去的邊，還原成圖示本身的外框。
            let (bx, by) = (minx + r, miny + r);
            let (bw, bh) = ((maxx + 1).saturating_sub(minx + 2 * r), (maxy + 1).saturating_sub(miny + 2 * r));
            if bw == 0 || bh == 0 {
                continue;
            }
            let (lw, lh) = (bw as f64 / scale, bh as f64 / scale);
            let aspect = lw / lh;
            let squarish = lw >= MIN_SIDE && lh >= MIN_SIDE && (MIN_ASPECT..=MAX_ASPECT).contains(&aspect);
            // 「⋮」「···」這類選單點點：一邊很細、另一邊夠長，但不能是 1px 的分隔線或底線。
            let (thin, long) = (lw.min(lh), lw.max(lh));
            let dots = (MIN_THIN..=MAX_THIN).contains(&thin) && long >= MIN_SIDE && long / thin <= MAX_THIN_RATIO;
            if lw > MAX_SIDE || lh > MAX_SIDE || !(squarish || dots) {
                continue;
            }
            // 貼著區域邊緣的多半是視窗邊框或被截斷的內容。
            if minx == 0 || miny == 0 || maxx + 1 >= cw || maxy + 1 >= ch {
                continue;
            }
            found.push(Rect { x: (x0 + bx) as f64, y: (y0 + by) as f64, w: bw as f64, h: bh as f64 });
        }
    }

    sort_reading_order(&mut found);
    found
}

/// 同一水平帶（y 中心差小於較矮者的一半高）由左到右，其餘由上到下。
pub fn sort_reading_order(rects: &mut [Rect]) {
    rects.sort_by(|a, b| {
        let (ay, by) = (a.y + a.h / 2.0, b.y + b.h / 2.0);
        if (ay - by).abs() < a.h.min(b.h) / 2.0 {
            a.x.total_cmp(&b.x)
        } else {
            ay.total_cmp(&by)
        }
    });
}

/// 方形結構元素的膨脹（先橫向再縱向，各自用滑動視窗，O(n)）。
fn dilate(mask: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    let mut tmp = vec![false; w * h];
    for y in 0..h {
        let row = &mask[y * w..(y + 1) * w];
        // prefix sum of set pixels
        let mut pre = vec![0u32; w + 1];
        for x in 0..w {
            pre[x + 1] = pre[x] + u32::from(row[x]);
        }
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r + 1).min(w));
            tmp[y * w + x] = pre[b] - pre[a] > 0;
        }
    }
    let mut out = vec![false; w * h];
    for x in 0..w {
        let mut pre = vec![0u32; h + 1];
        for y in 0..h {
            pre[y + 1] = pre[y] + u32::from(tmp[y * w + x]);
        }
        for y in 0..h {
            let (a, b) = (y.saturating_sub(r), (y + r + 1).min(h));
            out[y * w + x] = pre[b] - pre[a] > 0;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Img {
        w: usize,
        h: usize,
        lum: Vec<u8>,
    }

    impl Img {
        fn new(w: usize, h: usize, bg: u8) -> Img {
            Img { w, h, lum: vec![bg; w * h] }
        }
        fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, v: u8) {
            for yy in y..y + h {
                for xx in x..x + w {
                    self.lum[yy * self.w + xx] = v;
                }
            }
        }
        /// 空心方框（模擬線條式圖示，中間是背景）。
        fn outline(&mut self, x: usize, y: usize, size: usize, thickness: usize, v: u8) {
            self.fill(x, y, size, thickness, v);
            self.fill(x, y + size - thickness, size, thickness, v);
            self.fill(x, y, thickness, size, v);
            self.fill(x + size - thickness, y, thickness, size, v);
        }
    }

    fn centers(r: &[Rect]) -> Vec<(i32, i32)> {
        r.iter().map(|r| ((r.x + r.w / 2.0).round() as i32, (r.y + r.h / 2.0).round() as i32)).collect()
    }

    #[test]
    fn finds_four_header_icons_in_left_to_right_order_on_a_dark_theme() {
        let mut img = Img::new(500, 600, 30);
        for (i, x) in [380usize, 410, 440, 470].iter().enumerate() {
            img.outline(*x, 50, 16, 2, 230 - i as u8);
        }
        let found = detect_icons(&img.lum, img.w, img.h, 1.0, Region::Top);
        assert_eq!(centers(&found), vec![(388, 58), (418, 58), (448, 58), (478, 58)]);
    }

    #[test]
    fn works_on_a_light_theme_too() {
        let mut img = Img::new(400, 400, 245);
        img.outline(300, 30, 16, 2, 40);
        img.outline(340, 30, 16, 2, 40);
        assert_eq!(detect_icons(&img.lum, img.w, img.h, 1.0, Region::Top).len(), 2);
    }

    #[test]
    fn retina_images_report_pixel_coordinates_but_filter_by_logical_size() {
        let mut img = Img::new(1000, 1200, 30);
        img.outline(760, 100, 32, 4, 230); // 邏輯 16x16
        img.outline(820, 100, 32, 4, 230);
        let found = detect_icons(&img.lum, img.w, img.h, 2.0, Region::Top);
        assert_eq!(centers(&found), vec![(776, 116), (836, 116)]);
        // 同樣大小在 scale=1 時是 32 邏輯像素，仍在上限內；放大到 80px 的方塊則應被當成圖片而略過。
        let mut big = Img::new(1000, 1200, 30);
        big.outline(500, 100, 160, 6, 230);
        assert!(detect_icons(&big.lum, big.w, big.h, 2.0, Region::Top).is_empty());
    }

    #[test]
    fn ignores_wide_text_like_bars_big_pictures_and_specks() {
        let mut img = Img::new(500, 600, 30);
        img.fill(60, 50, 60, 14, 220); // 標題文字（太寬）
        img.fill(200, 20, 70, 70, 200); // 頭像（太大）
        img.fill(300, 55, 2, 2, 255); // 雜訊
        img.outline(400, 50, 16, 2, 230); // 真的圖示
        let found = detect_icons(&img.lum, img.w, img.h, 1.0, Region::Top);
        assert_eq!(centers(&found), vec![(408, 58)]);
    }

    #[test]
    fn menu_dots_are_icons_but_hairlines_and_underlines_are_not() {
        let mut img = Img::new(500, 600, 30);
        // 「⋮」：三個點，垂直排列（約 4 寬 x 16 高）
        img.fill(470, 50, 4, 4, 230);
        img.fill(470, 56, 4, 4, 230);
        img.fill(470, 62, 4, 4, 230);
        img.fill(300, 40, 1, 40, 200); // 1px 垂直分隔線
        img.fill(100, 70, 60, 1, 200); // 1px 底線
        let found = detect_icons(&img.lum, img.w, img.h, 1.0, Region::Top);
        assert_eq!(centers(&found), vec![(472, 58)], "{found:?}");
        // 「···」：水平排列
        let mut h = Img::new(500, 600, 30);
        h.fill(400, 50, 4, 4, 230);
        h.fill(406, 50, 4, 4, 230);
        h.fill(412, 50, 4, 4, 230);
        assert_eq!(detect_icons(&h.lum, h.w, h.h, 1.0, Region::Top).len(), 1);
    }

    #[test]
    fn a_text_sized_label_is_not_mistaken_for_menu_dots() {
        let mut img = Img::new(500, 600, 30);
        img.fill(100, 50, 28, 14, 220); // 兩個字寬的標題
        img.fill(200, 50, 60, 6, 220); // 又粗又長的色條
        assert!(detect_icons(&img.lum, img.w, img.h, 1.0, Region::Top).is_empty());
    }

    #[test]
    fn region_restricts_the_search_area() {
        let mut img = Img::new(500, 600, 30);
        img.outline(400, 40, 16, 2, 230); // 上方
        img.outline(400, 550, 16, 2, 230); // 下方
        assert_eq!(detect_icons(&img.lum, img.w, img.h, 1.0, Region::Top).len(), 1);
        assert_eq!(detect_icons(&img.lum, img.w, img.h, 1.0, Region::Bottom).len(), 1);
        assert_eq!(detect_icons(&img.lum, img.w, img.h, 1.0, Region::All).len(), 2);
        assert!(detect_icons(&img.lum, img.w, img.h, 1.0, Region::Left).is_empty());
    }

    #[test]
    fn a_blank_or_tiny_image_yields_nothing_instead_of_panicking() {
        assert!(detect_icons(&[], 0, 0, 1.0, Region::All).is_empty());
        assert!(detect_icons(&[10; 4], 2, 2, 1.0, Region::All).is_empty());
        let flat = Img::new(300, 300, 90);
        assert!(detect_icons(&flat.lum, flat.w, flat.h, 1.0, Region::All).is_empty());
        // 資料比宣告的尺寸短：不可越界
        assert!(detect_icons(&[0; 10], 300, 300, 1.0, Region::All).is_empty());
    }

    #[test]
    fn icons_in_the_same_band_are_ordered_by_x_even_if_their_heights_differ_slightly() {
        let mut img = Img::new(500, 600, 30);
        img.outline(440, 52, 16, 2, 230); // 右邊、稍低
        img.outline(400, 50, 16, 2, 230); // 左邊、稍高
        let found = detect_icons(&img.lum, img.w, img.h, 1.0, Region::Top);
        assert!(found[0].x < found[1].x);
    }

    #[test]
    fn region_parsing_accepts_aliases_and_rejects_garbage() {
        assert_eq!(Region::parse("Header").unwrap(), Region::Top);
        assert_eq!(Region::parse("footer").unwrap(), Region::Bottom);
        assert!(Region::parse("middle").is_err());
    }
}
