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

#[derive(Serialize)]
pub struct NpmMcpResult {
    pub qualified_name: String,
    pub display_name: String,
    pub description: String,
    pub homepage: Option<String>,
    pub npx_command: Option<String>,
    pub weekly_downloads: u64,
}

#[derive(Serialize)]
pub struct NpmMcpPage {
    pub results: Vec<NpmMcpResult>,
    pub total: u64,
}

/// Proxy npm registry search through the backend to avoid WebView fetch restrictions.
/// Appends "mcp" to the query to focus results on MCP packages.
#[tauri::command]
pub async fn search_npm_mcp(query: String, from: u64) -> Result<NpmMcpPage, String> {
    let client = reqwest::Client::builder()
        .user_agent("AITerm/1.0")
        .build()
        .map_err(|e| e.to_string())?;

    // Append "mcp" so results focus on MCP-related packages.
    let search_query = format!("{} mcp", query.trim());
    let url = format!(
        "https://registry.npmjs.org/-/v1/search?text={}&size=20&from={}",
        percent_encode(&search_query),
        from,
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

    // Collect package names for bulk download count fetch.
    let names: Vec<String> = objects.iter()
        .filter_map(|o| o["package"]["name"].as_str().map(String::from))
        .collect();

    // Fetch weekly download counts in a single bulk request.
    let downloads = if names.is_empty() {
        std::collections::HashMap::new()
    } else {
        let dl_url = format!(
            "https://api.npmjs.org/downloads/point/last-week/{}",
            names.join(","),
        );
        let dl_resp: serde_json::Value = async {
            let r = client.get(&dl_url).send().await?;
            r.json::<serde_json::Value>().await
        }
        .await
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        // Bulk response: { "pkg-name": { "downloads": N }, ... }
        // Single package response: { "downloads": N, "package": "name" }
        let mut map = std::collections::HashMap::new();
        if names.len() == 1 {
            if let Some(n) = dl_resp["downloads"].as_u64() {
                map.insert(names[0].clone(), n);
            }
        } else if let Some(obj) = dl_resp.as_object() {
            for (k, v) in obj {
                if let Some(n) = v["downloads"].as_u64() {
                    map.insert(k.clone(), n);
                }
            }
        }
        map
    };

    let results = objects.iter().filter_map(|o| {
        let pkg = &o["package"];
        let name = pkg["name"].as_str()?;
        let display_name = pkg["name"].as_str().unwrap_or(name).to_string();
        let description = pkg["description"].as_str().unwrap_or("").to_string();
        let homepage = pkg["links"]["homepage"].as_str()
            .or_else(|| pkg["links"]["repository"].as_str())
            .map(String::from);
        let npx_command = Some(format!("npx -y {name}"));
        let weekly_downloads = downloads.get(name).copied().unwrap_or(0);

        Some(NpmMcpResult {
            qualified_name: name.to_string(),
            display_name,
            description,
            homepage,
            npx_command,
            weekly_downloads,
        })
    }).collect();

    Ok(NpmMcpPage { results, total })
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
