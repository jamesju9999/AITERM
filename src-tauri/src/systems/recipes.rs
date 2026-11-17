//! 配方：AI 成功完成的一串操作，存起來之後直接重播。見
//! docs/superpowers/specs/2026-11-17-system-recipes-design.md。
//!
//! 元素參照（`e5`、`f1e5`）每次快照都不同，不能存。存的是「定位器」（角色＋名稱＋第幾個），
//! 重播時對新快照重新找出參照。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::{FromRow, SqlitePool};

/// 可以進配方的動作類工具（日誌只收這些）。
pub fn is_replayable(tool: &str) -> bool {
    matches!(
        tool,
        "sys_navigate" | "sys_click" | "sys_type" | "sys_fill_form" | "sys_press_key" | "sys_select_option" | "sys_drag" | "sys_handle_dialog"
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Locator {
    pub role: String,
    pub name: String,
    /// 在「同角色同名稱」的元素中的順位（0 起算）。
    pub nth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub role: String,
    pub name: String,
    pub reference: String,
}

/// 解析 Playwright 快照的 YAML 文字，抽出每個有參照的元素。
/// 行的形狀：`- button "送出訂單" [ref=f1e5]`、`- textbox [ref=e4]: "1"`、`- heading "x" [level=1] [ref=e2]`。
pub fn parse_snapshot(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim_start().trim_start_matches("- ");
        let Some(ref_pos) = t.find("[ref=") else { continue };
        let reference: String = t[ref_pos + 5..].chars().take_while(|c| *c != ']').collect();
        let head = &t[..ref_pos];
        let role: String = head.chars().take_while(|c| !c.is_whitespace() && *c != '"').collect();
        if role.is_empty() || reference.is_empty() {
            continue;
        }
        let name = match head.find('"') {
            Some(start) => {
                let rest = &head[start + 1..];
                let mut name = String::new();
                let mut esc = false;
                for c in rest.chars() {
                    if esc {
                        name.push(c);
                        esc = false;
                    } else if c == '\\' {
                        esc = true;
                    } else if c == '"' {
                        break;
                    } else {
                        name.push(c);
                    }
                }
                name
            }
            None => String::new(),
        };
        out.push(Entry { role, name, reference });
    }
    out
}

pub fn locator_for(entries: &[Entry], reference: &str) -> Option<Locator> {
    let idx = entries.iter().position(|e| e.reference == reference)?;
    let e = &entries[idx];
    let nth = entries[..idx].iter().filter(|x| x.role == e.role && x.name == e.name).count();
    Some(Locator { role: e.role.clone(), name: e.name.clone(), nth })
}

pub fn ref_for(entries: &[Entry], loc: &Locator) -> Option<String> {
    entries.iter().filter(|e| e.role == loc.role && e.name == loc.name).nth(loc.nth).map(|e| e.reference.clone())
}

fn is_ref_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k == "ref" || k.ends_with("target") || k.ends_with("ref")
}

/// 把 `args` 裡「參照欄位」的已知參照換成定位器。
pub fn to_locators(args: &Value, entries: &[Entry]) -> Value {
    match args {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| {
                    let nv = match v {
                        Value::String(s) if is_ref_key(k) => match locator_for(entries, s) {
                            Some(l) => json!({ "$locator": l }),
                            None => v.clone(),
                        },
                        _ => to_locators(v, entries),
                    };
                    (k.clone(), nv)
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|v| to_locators(v, entries)).collect()),
        other => other.clone(),
    }
}

/// 把定位器換回目前快照裡的參照；找不到就回錯誤（說明是哪個元素）。
pub fn resolve_locators(args: &Value, entries: &[Entry]) -> Result<Value, String> {
    match args {
        Value::Object(m) => {
            if let Some(l) = m.get("$locator") {
                let loc: Locator = serde_json::from_value(l.clone()).map_err(|e| e.to_string())?;
                return ref_for(entries, &loc)
                    .map(Value::String)
                    .ok_or_else(|| format!("畫面上找不到元素：{} 「{}」（第 {} 個）", loc.role, loc.name, loc.nth + 1));
            }
            let mut out = Map::new();
            for (k, v) in m {
                out.insert(k.clone(), resolve_locators(v, entries)?);
            }
            Ok(Value::Object(out))
        }
        Value::Array(a) => Ok(Value::Array(a.iter().map(|v| resolve_locators(v, entries)).collect::<Result<_, _>>()?)),
        other => Ok(other.clone()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    /// 錄製時用的實際值；存配方時把等於它的字串換成 `{{name}}`。
    pub example: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub tool: String,
    pub args: Value,
    pub intent: String,
}

/// 存配方時：字串值等於某參數的範例就換成佔位符；`url` 欄位用子字串取代（參數常在查詢字串裡）。
pub fn parameterize(args: &Value, params: &[Param]) -> Value {
    fn walk(v: &Value, key: Option<&str>, params: &[Param]) -> Value {
        match v {
            Value::String(s) => {
                let mut out = s.clone();
                for p in params.iter().filter(|p| !p.example.is_empty()) {
                    if key == Some("url") {
                        out = out.replace(&p.example, &format!("{{{{{}}}}}", p.name));
                    } else if out == p.example {
                        out = format!("{{{{{}}}}}", p.name);
                    }
                }
                Value::String(out)
            }
            Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), walk(x, Some(k), params))).collect()),
            Value::Array(a) => Value::Array(a.iter().map(|x| walk(x, key, params)).collect()),
            other => other.clone(),
        }
    }
    walk(args, None, params)
}

/// 重播時：把 `{{name}}` 換成提供的值；用到沒提供的參數就回錯誤。
pub fn apply_params(args: &Value, values: &HashMap<String, String>) -> Result<Value, String> {
    match args {
        Value::String(s) => {
            let mut out = String::new();
            let mut rest = s.as_str();
            while let Some(i) = rest.find("{{") {
                out.push_str(&rest[..i]);
                let Some(j) = rest[i..].find("}}") else {
                    out.push_str(&rest[i..]);
                    rest = "";
                    break;
                };
                let name = &rest[i + 2..i + j];
                match values.get(name) {
                    Some(v) => out.push_str(v),
                    None => return Err(format!("缺少參數：{name}")),
                }
                rest = &rest[i + j + 2..];
            }
            out.push_str(rest);
            Ok(Value::String(out))
        }
        Value::Object(m) => {
            let mut o = Map::new();
            for (k, v) in m {
                o.insert(k.clone(), apply_params(v, values)?);
            }
            Ok(Value::Object(o))
        }
        Value::Array(a) => Ok(Value::Array(a.iter().map(|v| apply_params(v, values)).collect::<Result<_, _>>()?)),
        other => Ok(other.clone()),
    }
}

/// 宣告了卻沒提供的參數名稱。
pub fn missing_params(params: &[Param], values: &HashMap<String, String>) -> Vec<String> {
    params.iter().filter(|p| !values.contains_key(&p.name)).map(|p| p.name.clone()).collect()
}

// ── 儲存（`systems.db` 的 system_recipes）──────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Recipe {
    pub id: i64,
    pub system_id: String,
    pub name: String,
    pub description: String,
    pub params: Vec<Param>,
    pub steps: Vec<Step>,
    pub ok_count: i64,
    pub fail_count: i64,
    pub last_run_ms: Option<i64>,
}

#[derive(FromRow)]
struct Row {
    id: i64,
    system_id: String,
    name: String,
    description: String,
    params: String,
    steps: String,
    ok_count: i64,
    fail_count: i64,
    last_run_ms: Option<i64>,
}

impl From<Row> for Recipe {
    fn from(r: Row) -> Self {
        Recipe {
            id: r.id,
            system_id: r.system_id,
            name: r.name,
            description: r.description,
            params: serde_json::from_str(&r.params).unwrap_or_default(),
            steps: serde_json::from_str(&r.steps).unwrap_or_default(),
            ok_count: r.ok_count,
            fail_count: r.fail_count,
            last_run_ms: r.last_run_ms,
        }
    }
}

pub async fn init(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS system_recipes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            system_id TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            params TEXT NOT NULL,
            steps TEXT NOT NULL,
            created_ms INTEGER NOT NULL,
            last_run_ms INTEGER,
            ok_count INTEGER NOT NULL DEFAULT 0,
            fail_count INTEGER NOT NULL DEFAULT 0,
            UNIQUE (system_id, name)
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// 新增；同系統同名會覆蓋（步驟換新、次數歸零）。
pub async fn save(pool: &SqlitePool, system_id: &str, name: &str, description: &str, params: &[Param], steps: &[Step]) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO system_recipes (system_id, name, description, params, steps, created_ms)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(system_id, name) DO UPDATE SET
           description = excluded.description, params = excluded.params, steps = excluded.steps,
           ok_count = 0, fail_count = 0, last_run_ms = NULL",
    )
    .bind(system_id)
    .bind(name)
    .bind(description)
    .bind(serde_json::to_string(params).unwrap_or_default())
    .bind(serde_json::to_string(steps).unwrap_or_default())
    .bind(now_ms())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(pool: &SqlitePool, system_id: &str) -> Result<Vec<Recipe>, sqlx::Error> {
    let rows = sqlx::query_as::<_, Row>(
        "SELECT id, system_id, name, description, params, steps, ok_count, fail_count, last_run_ms
         FROM system_recipes WHERE system_id = ? ORDER BY name",
    )
    .bind(system_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Recipe::from).collect())
}

pub async fn get(pool: &SqlitePool, system_id: &str, name: &str) -> Result<Option<Recipe>, sqlx::Error> {
    let row = sqlx::query_as::<_, Row>(
        "SELECT id, system_id, name, description, params, steps, ok_count, fail_count, last_run_ms
         FROM system_recipes WHERE system_id = ? AND name = ?",
    )
    .bind(system_id)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Recipe::from))
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM system_recipes WHERE id = ?").bind(id).execute(pool).await?.rows_affected() > 0)
}

pub async fn record_run(pool: &SqlitePool, id: i64, ok: bool) -> Result<(), sqlx::Error> {
    let sql = if ok {
        "UPDATE system_recipes SET ok_count = ok_count + 1, last_run_ms = ? WHERE id = ?"
    } else {
        "UPDATE system_recipes SET fail_count = fail_count + 1, last_run_ms = ? WHERE id = ?"
    };
    sqlx::query(sql).bind(now_ms()).bind(id).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNAP: &str = "### Page\n- Page URL: http://x/\n### Snapshot\n```yaml\n- generic [ref=f1e1]:\n  - heading \"測試訂單系統\" [level=1] [ref=f1e2]\n  - generic [ref=f1e3]:\n    - textbox [ref=f1e4]: \"1\"\n    - textbox [ref=f1e5]\n    - button \"送出訂單\" [ref=f1e6]\n    - button \"送出訂單\" [ref=f1e7]\n    - link \"say \\\"hi\\\"\" [ref=f1e8] [cursor=pointer]\n```";

    #[test]
    fn the_snapshot_is_parsed_into_role_name_and_ref() {
        let e = parse_snapshot(SNAP);
        let find = |r: &str| e.iter().find(|x| x.reference == r).unwrap().clone();
        assert_eq!((find("f1e2").role.as_str(), find("f1e2").name.as_str()), ("heading", "測試訂單系統"));
        assert_eq!((find("f1e4").role.as_str(), find("f1e4").name.as_str()), ("textbox", ""), "沒有名稱的元素");
        assert_eq!(find("f1e8").name, "say \"hi\"", "名稱裡的跳脫引號");
        assert!(e.iter().all(|x| !x.reference.is_empty()));
    }

    #[test]
    fn same_role_and_name_are_told_apart_by_order_and_survive_new_refs() {
        let e = parse_snapshot(SNAP);
        let l1 = locator_for(&e, "f1e6").unwrap();
        let l2 = locator_for(&e, "f1e7").unwrap();
        assert_eq!((l1.nth, l2.nth), (0, 1));
        // 換了一張快照（參照全變）：同一個定位器找到新的參照。
        let later = SNAP.replace("f1e", "f9e");
        let e2 = parse_snapshot(&later);
        assert_eq!(ref_for(&e2, &l1).as_deref(), Some("f9e6"));
        assert_eq!(ref_for(&e2, &l2).as_deref(), Some("f9e7"));
        let nameless = locator_for(&e, "f1e5").unwrap();
        assert_eq!((nameless.role.as_str(), nameless.nth), ("textbox", 1), "無名稱的 textbox 第二個");
        assert_eq!(ref_for(&e2, &nameless).as_deref(), Some("f9e5"));
    }

    #[test]
    fn only_reference_fields_become_locators_and_they_resolve_back() {
        let e = parse_snapshot(SNAP);
        let args = json!({"target":"f1e6","text":"f1e6","fields":[{"target":"f1e5","value":"abc"}],"intent":"寫：送出"});
        let loc = to_locators(&args, &e);
        assert!(loc["target"]["$locator"].is_object());
        assert_eq!(loc["text"], "f1e6", "不是參照欄位的字串不能被當成參照");
        assert!(loc["fields"][0]["target"]["$locator"].is_object());
        assert_eq!(loc["fields"][0]["value"], "abc");

        let e2 = parse_snapshot(&SNAP.replace("f1e", "f7e"));
        let back = resolve_locators(&loc, &e2).unwrap();
        assert_eq!(back["target"], "f7e6");
        assert_eq!(back["fields"][0]["target"], "f7e5");
    }

    #[test]
    fn a_missing_element_is_reported_by_name() {
        let e = parse_snapshot(SNAP);
        let loc = to_locators(&json!({"target":"f1e6"}), &e);
        let err = resolve_locators(&loc, &parse_snapshot("- heading \"別的頁面\" [ref=e1]")).unwrap_err();
        assert!(err.contains("送出訂單"), "{err}");
    }

    #[test]
    fn parameters_replace_examples_when_saving_and_placeholders_when_running() {
        let params = vec![Param { name: "order_no".into(), example: "A-102".into() }];
        let saved = parameterize(&json!({"text":"A-102","url":"http://x/o?id=A-102","other":"A-1020","n":5}), &params);
        assert_eq!(saved["text"], "{{order_no}}");
        assert_eq!(saved["url"], "http://x/o?id={{order_no}}", "url 用子字串取代");
        assert_eq!(saved["other"], "A-1020", "非 url 欄位只取代整個字串相等的");
        assert_eq!(saved["n"], 5);

        let values: HashMap<String, String> = [("order_no".to_string(), "B-7".to_string())].into();
        let run = apply_params(&saved, &values).unwrap();
        assert_eq!(run["text"], "B-7");
        assert_eq!(run["url"], "http://x/o?id=B-7");
        assert_eq!(apply_params(&saved, &HashMap::new()).unwrap_err(), "缺少參數：order_no");
        assert_eq!(missing_params(&params, &HashMap::new()), vec!["order_no".to_string()]);
        assert!(missing_params(&params, &values).is_empty());
    }

    #[test]
    fn only_acting_tools_are_replayable() {
        assert!(is_replayable("sys_click") && is_replayable("sys_navigate"));
        assert!(!is_replayable("sys_snapshot") && !is_replayable("sys_recipe_run") && !is_replayable("sys_take_screenshot"));
    }

    async fn pool() -> SqlitePool {
        let p = SqlitePool::connect("sqlite::memory:").await.unwrap();
        init(&p).await.unwrap();
        p
    }

    fn step(tool: &str) -> Step {
        Step { tool: tool.into(), args: json!({"a":1}), intent: "寫：x".into() }
    }

    #[tokio::test]
    async fn recipes_are_saved_listed_overwritten_and_deleted_per_system() {
        let p = pool().await;
        save(&p, "s1", "送出訂單", "說明", &[Param { name: "n".into(), example: "1".into() }], &[step("sys_click")]).await.unwrap();
        save(&p, "s2", "送出訂單", "", &[], &[step("sys_type")]).await.unwrap();
        let l = list(&p, "s1").await.unwrap();
        assert_eq!(l.len(), 1, "別的系統的不能混進來");
        assert_eq!(l[0].steps[0].tool, "sys_click");
        assert_eq!(l[0].params[0].name, "n");

        record_run(&p, l[0].id, true).await.unwrap();
        record_run(&p, l[0].id, false).await.unwrap();
        let g = get(&p, "s1", "送出訂單").await.unwrap().unwrap();
        assert_eq!((g.ok_count, g.fail_count), (1, 1));
        assert!(g.last_run_ms.is_some());

        save(&p, "s1", "送出訂單", "新說明", &[], &[step("sys_type"), step("sys_click")]).await.unwrap();
        let g = get(&p, "s1", "送出訂單").await.unwrap().unwrap();
        assert_eq!((g.steps.len(), g.ok_count, g.fail_count), (2, 0, 0), "同名覆蓋、次數歸零");
        assert_eq!(list(&p, "s1").await.unwrap().len(), 1);

        assert!(delete(&p, g.id).await.unwrap());
        assert!(get(&p, "s1", "送出訂單").await.unwrap().is_none());
        assert!(!delete(&p, g.id).await.unwrap());
    }
}
