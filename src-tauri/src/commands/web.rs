use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde::Serialize;

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            b => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[tauri::command]
pub async fn web_search(query: String) -> Result<String, String> {
    let encoded = percent_encode(&query);
    let url = format!("https://lite.duckduckgo.com/lite/?q={}", encoded);

    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        ),
    );

    let client = reqwest::Client::new();
    let html = client
        .get(&url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;

    let results = parse_ddg_results(&html);
    if results.is_empty() {
        return Ok("No results found.".to_string());
    }

    let formatted = results
        .iter()
        .enumerate()
        .map(|(i, (title, link, snippet))| {
            format!("[{}] {}\nURL: {}\n{}\n", i + 1, title, link, snippet)
        })
        .collect::<Vec<_>>()
        .join("\n");

    Ok(formatted)
}

fn parse_ddg_results(html: &str) -> Vec<(String, String, String)> {
    let mut results = Vec::new();

    // Find all <a class="result-link" href="...">title</a> occurrences
    let mut pos = 0;
    while results.len() < 5 {
        // Find the next result-link anchor
        let marker = "class=\"result-link\"";
        let Some(class_pos) = html[pos..].find(marker) else { break };
        let tag_start = match html[..pos + class_pos].rfind('<') {
            Some(p) => p,
            None => { pos += class_pos + marker.len(); continue; }
        };

        // Extract href
        let tag_end = (pos + class_pos + marker.len() + 200).min(html.len());
        let tag_region = &html[tag_start..tag_end];
        let href = extract_attr(tag_region, "href").unwrap_or_default();

        // Extract link text (content between > and </a>)
        let Some(close_bracket) = html[tag_start..].find('>') else {
            pos += class_pos + marker.len();
            continue;
        };
        let content_start = tag_start + close_bracket + 1;
        let title = if let Some(end_tag) = html[content_start..].find("</a>") {
            strip_tags(&html[content_start..content_start + end_tag]).trim().to_string()
        } else {
            String::new()
        };

        // Find the snippet: look for the next <td class="result-snippet"> after this link
        let snippet_marker = "class=\"result-snippet\"";
        let snippet = if let Some(snip_pos) = html[pos + class_pos..].find(snippet_marker) {
            let abs = pos + class_pos + snip_pos;
            if let Some(open) = html[abs..].find('>') {
                let snip_start = abs + open + 1;
                if let Some(close) = html[snip_start..].find("</td>") {
                    strip_tags(&html[snip_start..snip_start + close]).trim().to_string()
                } else { String::new() }
            } else { String::new() }
        } else { String::new() };

        if !href.is_empty() && !title.is_empty() {
            results.push((title, href, snippet));
        }

        pos += class_pos + marker.len();
    }

    results
}

fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let search = format!("{}=\"", attr);
    let start = tag.find(&search)? + search.len();
    let end = tag[start..].find('"')?;
    Some(tag[start..start + end].to_string())
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

// ── npm MCP marketplace search ────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct NpmMcpResult {
    pub qualified_name: String,
    pub display_name: String,
    pub description: String,
    pub homepage: Option<String>,
    pub npx_command: Option<String>,
    pub weekly_downloads: u64,
}

#[derive(Serialize, Clone)]
pub struct NpmMcpPage {
    pub results: Vec<NpmMcpResult>,
    pub total: u64,
}

/// Proxy npm registry search through the backend to avoid WebView fetch restrictions.
#[tauri::command]
pub async fn npm_mcp_search(query: String, offset: u64) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent("AITerm/1.0")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let search_query = format!("{} mcp", query.trim());
    let url = format!(
        "https://registry.npmjs.org/-/v1/search?text={}&size=20&from={}",
        percent_encode(&search_query),
        offset,
    );

    let resp: serde_json::Value = client
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let total = resp["total"].as_u64().unwrap_or(0);
    let objects = resp["objects"].as_array().cloned().unwrap_or_default();

    let mut results: Vec<NpmMcpResult> = objects.iter().filter_map(|o| {
        let pkg = &o["package"];
        let name = pkg["name"].as_str()?;
        Some(NpmMcpResult {
            qualified_name: name.to_string(),
            display_name: name.to_string(),
            description: pkg["description"].as_str().unwrap_or("").to_string(),
            homepage: pkg["links"]["homepage"].as_str()
                .or_else(|| pkg["links"]["repository"].as_str())
                .map(String::from),
            npx_command: Some(format!("npx -y {name}")),
            weekly_downloads: 0,
        })
    }).collect();

    // Fetch weekly downloads — scoped packages (@org/name) are not supported
    // by the npm bulk downloads API, so only query unscoped packages.
    let unscoped: Vec<String> = results.iter()
        .filter(|r| !r.qualified_name.starts_with('@'))
        .map(|r| r.qualified_name.clone())
        .collect();

    if !unscoped.is_empty() {
        // Bulk API: comma-separated package names
        let names_param = unscoped.join(",");
        let url = format!(
            "https://api.npmjs.org/downloads/point/last-week/{}",
            names_param,
        );
        if let Ok(resp) = client.get(&url).send().await {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                for r in results.iter_mut() {
                    if !r.qualified_name.starts_with('@') {
                        if let Some(count) = data[&r.qualified_name]["downloads"].as_u64() {
                            r.weekly_downloads = count;
                        }
                    }
                }
            }
        }
    }

    let page = NpmMcpPage { results, total };
    serde_json::to_string(&page).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn web_fetch(url: String) -> Result<String, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        ),
    );

    let client = reqwest::Client::new();
    let html = client
        .get(&url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;

    let text = strip_tags(&html);
    // Collapse excess whitespace
    let text: String = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    let truncated = if text.len() > 3000 {
        text[..3000].to_string()
    } else {
        text
    };

    Ok(truncated)
}
