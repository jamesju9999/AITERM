//! 從 AI 命令列工具自己寫在磁碟上的對話記錄，取出「最近兩輪」給下一步建議用。
//!
//! 為什麼需要：Claude Code 的 `"tui": "fullscreen"` 跑在 alternate buffer，xterm 只有
//! 看得到的那幾列；一輪回覆比一屏長時，上面的內容只在工具自己的記憶體裡。唯一完整
//! 的來源是它即時寫的 JSONL。Codex 同理（`~/.codex/sessions/**/rollout-*.jsonl`）。
//!
//! 認出是哪個 session 不靠路徑或 cwd：拿每個候選檔最後兩輪的文字，去比對目前畫面上
//! 看得到的內容，重合最多的就是。同資料夾開好幾個 session 也不會認錯，也沒有
//! Windows 路徑編碼的問題。對不上（0 分）就回 None，不猜。
//!
//! 兩種格式都沒有官方文件；解析不了的行一律跳過，整份解析不出東西就回 None，
//! 前端退回只讀畫面。
//!
//! 見 docs/superpowers/specs/2026-10-15-suggestion-full-context-design.md。

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use serde_json::Value;

/// 只看這麼久內有更新的記錄檔——正在跑的工具每一輪都會寫入。
const RECENT: Duration = Duration::from_secs(15 * 60);
/// 先讀尾端這麼多位元組；記錄檔可以長到好幾 MB。
const TAIL_BYTES: u64 = 256 * 1024;
/// 尾端裡湊不到兩則使用者提問就依序讀更多。一輪 agent 工作可以有好幾 MB 的工具呼叫
/// （實測 520 KB 之前才找到上一則提問）。
const TAIL_STEPS: [u64; 4] = [TAIL_BYTES, 1024 * 1024, 4 * 1024 * 1024, 16 * 1024 * 1024];
/// 渲染超過上限時，最新那則提問最多保留這麼多字元。
const MAX_PROMPT_KEEP_CHARS: usize = 1500;
/// 渲染時中間被省略的標記。
const OMITTED: &str = "…（中間省略）…\n";
/// 一整行工具呼叫的上限（字元）。
const TOOL_LINE_MAX_CHARS: usize = 300;
/// 渲染結果上限（字元），超過從前面截。
const MAX_RENDER_CHARS: usize = 8000;
/// 工具呼叫每個參數的上限（字元）。
const TOOL_ARG_MAX_CHARS: usize = 200;
/// 正規化後至少這麼長的行才拿來比對，太短的（「完成了」）到處都對得上。
const MIN_MATCH_CHARS: usize = 8;
/// Codex 的 sessions 是 YYYY/MM/DD 三層，再多一層當餘裕。
const MAX_WALK_DEPTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    Claude,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    role: Role,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptContext {
    pub tool: Tool,
    pub text: String,
}

fn truncate_chars(s: &str, max: usize) -> String {
    let total = s.chars().count();
    if total <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max).collect();
    format!("{kept}…")
}

/// `name(key=value, …)`，每個參數截到 TOOL_ARG_MAX_CHARS。
fn tool_line(name: &str, input: Option<&Value>) -> String {
    let args = match input {
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| {
                let raw = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("{k}={}", truncate_chars(&raw.replace('\n', " "), TOOL_ARG_MAX_CHARS))
            })
            .collect::<Vec<_>>()
            .join(", "),
        Some(Value::String(s)) => truncate_chars(&s.replace('\n', " "), TOOL_ARG_MAX_CHARS),
        _ => String::new(),
    };
    truncate_chars(&format!("{name}({args})"), TOOL_LINE_MAX_CHARS)
}

/// Claude Code 自己塞進使用者訊息的東西（斜線指令的 caveat、system reminder），不是使用者說的話。
fn is_claude_injected(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("<command-") || t.starts_with("<local-command") || t.starts_with("<system-reminder>")
}

fn parse_claude(jsonl: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for line in jsonl.lines() {
        let Ok(rec) = serde_json::from_str::<Value>(line.trim()) else { continue };
        if rec.get("isMeta").and_then(Value::as_bool) == Some(true)
            || rec.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let kind = rec.get("type").and_then(Value::as_str);
        let Some(content) = rec.pointer("/message/content") else { continue };
        match kind {
            Some("user") => {
                let texts: Vec<&str> = match content {
                    Value::String(s) => vec![s.as_str()],
                    Value::Array(blocks) => blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|b| b.get("text").and_then(Value::as_str))
                        .collect(),
                    _ => vec![],
                };
                for t in texts {
                    if !t.trim().is_empty() && !is_claude_injected(t) {
                        out.push(Entry { role: Role::User, text: t.trim().to_string() });
                    }
                }
            }
            Some("assistant") => {
                for b in content.as_array().into_iter().flatten() {
                    match b.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(t) = b.get("text").and_then(Value::as_str) {
                                if !t.trim().is_empty() {
                                    out.push(Entry { role: Role::Assistant, text: t.trim().to_string() });
                                }
                            }
                        }
                        Some("tool_use") => {
                            let name = b.get("name").and_then(Value::as_str).unwrap_or("?");
                            out.push(Entry { role: Role::Tool, text: tool_line(name, b.get("input")) });
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn parse_codex(jsonl: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for line in jsonl.lines() {
        let Ok(rec) = serde_json::from_str::<Value>(line.trim()) else { continue };
        let Some(p) = rec.get("payload") else { continue };
        let ptype = p.get("type").and_then(Value::as_str);
        match (rec.get("type").and_then(Value::as_str), ptype) {
            // 用 event_msg 而不是 response_item 的 message：後者的 user 訊息混著 <environment_context>。
            (Some("event_msg"), Some("user_message")) => {
                if let Some(t) = p.get("message").and_then(Value::as_str) {
                    if !t.trim().is_empty() {
                        out.push(Entry { role: Role::User, text: t.trim().to_string() });
                    }
                }
            }
            (Some("event_msg"), Some("agent_message")) => {
                if let Some(t) = p.get("message").and_then(Value::as_str) {
                    if !t.trim().is_empty() {
                        out.push(Entry { role: Role::Assistant, text: t.trim().to_string() });
                    }
                }
            }
            (Some("response_item"), Some("function_call")) => {
                let name = p.get("name").and_then(Value::as_str).unwrap_or("?");
                // arguments 是 JSON 字串；解析得了就逐個參數列，否則整段當一個參數。
                let args = p.get("arguments").and_then(Value::as_str).unwrap_or("");
                let parsed = serde_json::from_str::<Value>(args).ok();
                let input = parsed.or_else(|| Some(Value::String(args.to_string())));
                out.push(Entry { role: Role::Tool, text: tool_line(name, input.as_ref()) });
            }
            (Some("response_item"), Some("custom_tool_call")) => {
                let name = p.get("name").and_then(Value::as_str).unwrap_or("?");
                out.push(Entry { role: Role::Tool, text: tool_line(name, p.get("input")) });
            }
            _ => {}
        }
    }
    out
}

/// 從倒數第二則使用者提問起（不足兩則就從頭）。
fn last_two_turns(entries: &[Entry]) -> &[Entry] {
    let users: Vec<usize> = entries.iter().enumerate().filter(|(_, e)| e.role == Role::User).map(|(i, _)| i).collect();
    let start = if users.len() >= 2 { users[users.len() - 2] } else { 0 };
    &entries[start..]
}

/// 去掉空白與 markdown 符號——終端機會換行、TUI 會把 `**粗體**` 渲染掉。
fn normalize(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace() && !"*_#>|-`".contains(*c)).collect()
}

/// 最後兩輪的使用者／AI 文字裡，正規化後夠長、且出現在畫面上的行數。
fn match_score(entries: &[Entry], normalized_screen: &str) -> usize {
    last_two_turns(entries)
        .iter()
        .filter(|e| e.role != Role::Tool)
        .flat_map(|e| e.text.lines())
        .map(normalize)
        .filter(|l| l.chars().count() >= MIN_MATCH_CHARS && normalized_screen.contains(l.as_str()))
        .count()
}

fn render_entries(entries: &[Entry]) -> String {
    let mut out = String::new();
    for e in entries {
        let label = match e.role {
            Role::User => "使用者：",
            Role::Assistant => "AI：",
            Role::Tool => "〔工具〕",
        };
        out.push_str(label);
        out.push_str(&e.text);
        out.push('\n');
    }
    out
}

fn keep_tail(s: &str, max: usize) -> String {
    let total = s.chars().count();
    if total <= max {
        return s.to_string();
    }
    s.chars().skip(total - max).collect()
}

/// 最近兩輪。超過 MAX_RENDER_CHARS 時：最新那則提問一定放最前面（截到
/// MAX_PROMPT_KEEP_CHARS），中間標示省略，剩下的額度給最後面——
/// 只留尾端的話，一輪做很多事時提問本身會被切掉。
fn render(entries: &[Entry]) -> String {
    let turns = last_two_turns(entries);
    let full = render_entries(turns);
    if full.chars().count() <= MAX_RENDER_CHARS {
        return full;
    }
    let Some(u) = turns.iter().rposition(|e| e.role == Role::User) else {
        return keep_tail(&full, MAX_RENDER_CHARS);
    };
    let prompt = Entry { role: Role::User, text: truncate_chars(&turns[u].text, MAX_PROMPT_KEEP_CHARS) };
    let head = format!("{}{OMITTED}", render_entries(std::slice::from_ref(&prompt)));
    let rest = render_entries(&turns[u + 1..]);
    let budget = MAX_RENDER_CHARS.saturating_sub(head.chars().count());
    format!("{head}{}", keep_tail(&rest, budget))
}

/// 讀檔案尾端 `bytes`；有從中間開始讀時丟掉被切斷的第一行。
fn read_tail(path: &Path, bytes: u64) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(bytes);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    if start == 0 {
        return Some(text);
    }
    text.find('\n').map(|i| text[i + 1..].to_string())
}

/// 依 TAIL_STEPS 讀越來越多的尾端，直到有兩則使用者提問或已讀完整個檔案。
fn read_entries(path: &Path, tool: Tool) -> Option<Vec<Entry>> {
    let len = std::fs::metadata(path).ok()?.len();
    let mut entries = Vec::new();
    for bytes in TAIL_STEPS {
        let text = read_tail(path, bytes)?;
        entries = match tool {
            Tool::Claude => parse_claude(&text),
            Tool::Codex => parse_codex(&text),
        };
        if bytes >= len || entries.iter().filter(|e| e.role == Role::User).count() >= 2 {
            break;
        }
    }
    Some(entries)
}

/// `root` 底下（最多 MAX_WALK_DEPTH 層）`now - RECENT` 之後修改過的 `.jsonl`，附修改時間。
fn recent_jsonl(root: &Path, now: SystemTime) -> Vec<(PathBuf, SystemTime)> {
    fn walk(dir: &Path, depth: usize, now: SystemTime, out: &mut Vec<(PathBuf, SystemTime)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for entry in rd.filter_map(Result::ok) {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if depth < MAX_WALK_DEPTH {
                    walk(&path, depth + 1, now, out);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Ok(m) = meta.modified() {
                    if now.duration_since(m).map_or(true, |age| age <= RECENT) {
                        out.push((path, m));
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 1, now, &mut out);
    out
}

/// 在各工具的根目錄裡找出與畫面最吻合的 session，渲染它最近兩輪。
fn find_context(roots: &[(Tool, PathBuf)], screen: &str, now: SystemTime) -> Option<TranscriptContext> {
    let screen = normalize(screen);
    if screen.is_empty() {
        return None;
    }
    let mut best: Option<(usize, SystemTime, Tool, Vec<Entry>)> = None;
    for (tool, root) in roots {
        for (path, mtime) in recent_jsonl(root, now) {
            let Some(entries) = read_entries(&path, *tool) else { continue };
            let score = match_score(&entries, &screen);
            if score == 0 {
                continue;
            }
            let better = match &best {
                None => true,
                Some((s, m, _, _)) => score > *s || (score == *s && mtime > *m),
            };
            if better {
                best = Some((score, mtime, *tool, entries));
            }
        }
    }
    best.map(|(_, _, tool, entries)| TranscriptContext { tool, text: render(&entries) })
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn default_roots() -> Vec<(Tool, PathBuf)> {
    let home = dirs::home_dir();
    let mut roots = Vec::new();
    if let Some(claude) = env_dir("CLAUDE_CONFIG_DIR").or_else(|| home.as_ref().map(|h| h.join(".claude"))) {
        roots.push((Tool::Claude, claude.join("projects")));
    }
    if let Some(codex) = env_dir("CODEX_HOME").or_else(|| home.as_ref().map(|h| h.join(".codex"))) {
        roots.push((Tool::Codex, codex.join("sessions")));
    }
    roots
}

/// 依目前畫面找出對應的 AI 工具對話記錄最近兩輪。找不到、對不上都回 None。
#[tauri::command]
pub async fn agent_transcript_context(screen: String) -> Option<TranscriptContext> {
    tauri::async_runtime::spawn_blocking(move || find_context(&default_roots(), &screen, SystemTime::now()))
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn claude_line(v: Value) -> String {
        v.to_string()
    }

    fn claude_fixture() -> String {
        [
            claude_line(serde_json::json!({"type":"user","message":{"content":"第一個問題：幫我看看 parse 函式"}})),
            claude_line(serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"parse 函式看起來沒問題"}]}})),
            claude_line(serde_json::json!({"type":"user","isMeta":true,"message":{"content":"meta 訊息不該出現"}})),
            claude_line(serde_json::json!({"type":"user","message":{"content":[{"type":"text","text":"<command-name>/clear</command-name>"}]}})),
            claude_line(serde_json::json!({"type":"user","message":{"content":"第二個問題：請修正排序錯誤"}})),
            claude_line(serde_json::json!({"type":"assistant","message":{"content":[
                {"type":"thinking","thinking":"思考內容不該出現"},
                {"type":"tool_use","name":"Edit","input":{"file_path":"src/sort.ts","old_string":"a\nb"}}
            ]}})),
            claude_line(serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","content":"SECRET 檔案內容不該出現"}]}})),
            claude_line(serde_json::json!({"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"子代理不該出現"}]}})),
            claude_line(serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"**已修正**排序的比較函式，\n接下來建議補上邊界測試案例"}]}})),
            "not json".to_string(),
        ]
        .join("\n")
    }

    #[test]
    fn claude_keeps_prompts_replies_and_tool_calls_but_not_results_or_injected_text() {
        let e = parse_claude(&claude_fixture());
        let texts: Vec<&str> = e.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts.len(), 5, "{texts:?}");
        assert_eq!(e[0], Entry { role: Role::User, text: "第一個問題：幫我看看 parse 函式".into() });
        assert_eq!(e[3].role, Role::Tool);
        assert_eq!(e[3].text, "Edit(file_path=src/sort.ts, old_string=a b)");
        let joined = texts.join("|");
        for absent in ["meta", "/clear", "思考", "SECRET", "子代理"] {
            assert!(!joined.contains(absent), "{absent} leaked: {joined}");
        }
    }

    fn codex_fixture() -> String {
        [
            serde_json::json!({"type":"session_meta","payload":{"cwd":"/x"}}),
            serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>不該出現</environment_context>"}]}}),
            serde_json::json!({"type":"event_msg","payload":{"type":"user_message","message":"規劃 Gmail 整理工具\n"}}),
            serde_json::json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"ls -la\"}"}}),
            serde_json::json!({"type":"response_item","payload":{"type":"function_call_output","output":"OUTPUT 不該出現"}}),
            serde_json::json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","input":"*** Begin Patch\n*** Update File: a.py"}}),
            serde_json::json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"重複的回覆"}]}}),
            serde_json::json!({"type":"event_msg","payload":{"type":"agent_message","message":"下面是一個可落地的規劃"}}),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
    }

    #[test]
    fn codex_uses_event_messages_and_tool_calls_only() {
        let e = parse_codex(&codex_fixture());
        assert_eq!(
            e,
            vec![
                Entry { role: Role::User, text: "規劃 Gmail 整理工具".into() },
                Entry { role: Role::Tool, text: "exec_command(cmd=ls -la)".into() },
                Entry { role: Role::Tool, text: "apply_patch(*** Begin Patch *** Update File: a.py)".into() },
                Entry { role: Role::Assistant, text: "下面是一個可落地的規劃".into() },
            ]
        );
    }

    #[test]
    fn codex_unparseable_arguments_are_kept_as_one_argument() {
        let line = serde_json::json!({"type":"response_item","payload":{"type":"function_call","name":"f","arguments":"not json"}});
        assert_eq!(parse_codex(&line.to_string())[0].text, "f(not json)");
    }

    #[test]
    fn tool_arguments_are_truncated() {
        let long = "x".repeat(TOOL_ARG_MAX_CHARS + 50);
        let line = tool_line("W", Some(&serde_json::json!({ "content": long })));
        assert_eq!(line, format!("W(content={}…)", "x".repeat(TOOL_ARG_MAX_CHARS)));
    }

    #[test]
    fn render_starts_at_the_second_to_last_prompt_and_keeps_the_tail_when_long() {
        let mut e = vec![
            Entry { role: Role::User, text: "很久以前".into() },
            Entry { role: Role::User, text: "上一輪".into() },
            Entry { role: Role::Assistant, text: "上一輪回覆".into() },
            Entry { role: Role::User, text: "這一輪".into() },
        ];
        let r = render(&e);
        assert!(!r.contains("很久以前"));
        assert!(r.starts_with("使用者：上一輪\nAI：上一輪回覆\n使用者：這一輪"));

        e.push(Entry { role: Role::Assistant, text: format!("{}結尾", "長".repeat(MAX_RENDER_CHARS)) });
        let r = render(&e);
        assert!(r.chars().count() <= MAX_RENDER_CHARS, "{}", r.chars().count());
        assert!(r.ends_with("結尾\n"));
        // 一輪很長時，最新的提問本身不能被切掉。
        assert!(r.starts_with("使用者：這一輪\n"), "{}", r.chars().take(40).collect::<String>());
        assert!(r.contains(OMITTED));
    }

    #[test]
    fn a_very_long_latest_prompt_is_capped_but_still_leads() {
        let e = vec![
            Entry { role: Role::User, text: format!("開頭{}", "問".repeat(MAX_PROMPT_KEEP_CHARS * 2)) },
            Entry { role: Role::Assistant, text: format!("{}結尾", "答".repeat(MAX_RENDER_CHARS)) },
        ];
        let r = render(&e);
        assert!(r.starts_with("使用者：開頭"));
        assert!(r.chars().count() <= MAX_RENDER_CHARS);
        assert!(r.ends_with("結尾\n"));
    }

    #[test]
    fn a_whole_tool_line_is_capped() {
        let input = serde_json::json!({ "a": "x".repeat(150), "b": "y".repeat(150), "c": "z".repeat(150) });
        assert!(tool_line("Edit", Some(&input)).chars().count() <= TOOL_LINE_MAX_CHARS + 1);
    }

    #[test]
    fn reads_further_back_until_it_has_two_prompts() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("long.jsonl");
        let user = |t: &str| serde_json::json!({"type":"user","message":{"content":t}}).to_string();
        let filler = serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"x".repeat(10_000)}]}}).to_string();
        let mut lines = vec![user("上一輪的提問"), user("這一輪的提問")];
        // 兩則提問之後還有 ~600 KB 的回覆，第一次只讀 256 KB 一定看不到它們。
        lines.extend(std::iter::repeat(filler).take(60));
        fs::write(&p, lines.join("\n")).unwrap();
        let e = read_entries(&p, Tool::Claude).unwrap();
        let users: Vec<_> = e.iter().filter(|x| x.role == Role::User).map(|x| x.text.as_str()).collect();
        assert_eq!(users, vec!["上一輪的提問", "這一輪的提問"]);
    }

    #[test]
    fn render_with_a_single_prompt_starts_from_the_top() {
        let e = vec![
            Entry { role: Role::Assistant, text: "開場".into() },
            Entry { role: Role::User, text: "唯一的提問".into() },
        ];
        assert!(render(&e).starts_with("AI：開場"));
    }

    #[test]
    fn score_ignores_wrapping_markdown_and_short_lines() {
        let e = parse_claude(&claude_fixture());
        // 畫面把一行拆成兩行、粗體被渲染掉、前面多了 TUI 的符號。
        let screen = normalize("⏺ 已修正排序的比較\n  函式，\n  接下來建議補上邊界測試案例\n完成了");
        assert_eq!(match_score(&e, &screen), 2);
        assert_eq!(match_score(&e, &normalize("完全不相干的畫面內容在這裡")), 0);
        // 最後兩輪以外的不算。
        let mut older = vec![Entry { role: Role::User, text: "更早一輪的提問內容".into() }];
        older.extend(e.iter().cloned());
        assert_eq!(match_score(&older, &normalize("更早一輪的提問內容")), 0);
        assert_eq!(match_score(&older, &normalize("第一個問題：幫我看看 parse 函式")), 1);
        // 工具呼叫不拿來比對。
        assert_eq!(match_score(&e, &normalize("Edit(file_path=src/sort.ts, old_string=a b)")), 0);
    }

    #[test]
    fn short_lines_never_count() {
        let e = vec![Entry { role: Role::User, text: "1234567".into() }];
        assert_eq!(match_score(&e, "1234567"), 0);
        let e = vec![Entry { role: Role::User, text: "12345678".into() }];
        assert_eq!(match_score(&e, "12345678"), 1);
    }

    #[test]
    fn read_tail_drops_the_cut_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.jsonl");
        let first = "x".repeat(TAIL_BYTES as usize);
        fs::write(&p, format!("{first}\nsecond\nthird\n")).unwrap();
        assert_eq!(read_tail(&p, TAIL_BYTES).unwrap(), "second\nthird\n");
        fs::write(&p, "short\nfile\n").unwrap();
        assert_eq!(read_tail(&p, TAIL_BYTES).unwrap(), "short\nfile\n");
    }

    fn touch(path: &Path, body: &str, mtime: SystemTime) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
        fs::File::options().write(true).open(path).unwrap().set_modified(mtime).unwrap();
    }

    #[test]
    fn finds_the_session_whose_text_is_on_screen_across_tools() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let claude = dir.path().join("claude");
        let codex = dir.path().join("codex");
        touch(&claude.join("proj-a/s1.jsonl"), &claude_fixture(), now);
        // 同資料夾另一個 session，內容不在畫面上，即使比較新也不選。
        let other = serde_json::json!({"type":"user","message":{"content":"另一個 session 的提問內容"}}).to_string();
        touch(&claude.join("proj-a/s2.jsonl"), &other, now + Duration::from_secs(1));
        touch(&codex.join("2026/10/15/rollout-1.jsonl"), &codex_fixture(), now);
        let roots = vec![(Tool::Claude, claude.clone()), (Tool::Codex, codex.clone())];

        let ctx = find_context(&roots, "接下來建議補上邊界測試案例", now).unwrap();
        assert_eq!(ctx.tool, Tool::Claude);
        assert!(ctx.text.contains("第二個問題：請修正排序錯誤"));

        let ctx = find_context(&roots, "• 下面是一個可落地的規劃", now).unwrap();
        assert_eq!(ctx.tool, Tool::Codex);

        assert_eq!(find_context(&roots, "沒有任何記錄對得上的畫面", now), None);
        assert_eq!(find_context(&roots, "   ", now), None);
    }

    #[test]
    fn ties_go_to_the_newer_file_and_stale_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let root = dir.path().join("claude");
        let body = |reply: &str| {
            [
                serde_json::json!({"type":"user","message":{"content":"共同的提問內容在這裡"}}).to_string(),
                serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":reply}]}}).to_string(),
            ]
            .join("\n")
        };
        touch(&root.join("p/old.jsonl"), &body("舊的回覆內容"), now - Duration::from_secs(60));
        touch(&root.join("p/new.jsonl"), &body("新的回覆內容"), now);
        let roots = vec![(Tool::Claude, root.clone())];
        let ctx = find_context(&roots, "共同的提問內容在這裡", now).unwrap();
        assert!(ctx.text.contains("新的回覆內容"), "{}", ctx.text);

        // 只剩超過 RECENT 的舊檔：不採用。
        fs::remove_file(root.join("p/new.jsonl")).unwrap();
        touch(&root.join("p/old.jsonl"), &body("舊的回覆內容"), now - RECENT - Duration::from_secs(5));
        assert_eq!(find_context(&roots, "共同的提問內容在這裡", now), None);
    }

    #[test]
    fn missing_roots_are_fine() {
        let roots = vec![(Tool::Claude, PathBuf::from("/definitely/not/here"))];
        assert_eq!(find_context(&roots, "任何畫面內容都可以", SystemTime::now()), None);
    }
}
