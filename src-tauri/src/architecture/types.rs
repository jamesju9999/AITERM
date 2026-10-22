use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanFile {
    /// 相對專案根、一律用 `/` 分隔（跨平台一致）。
    pub path: String,
    pub lang: String,
    pub lines: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanResult {
    pub root: String,
    pub files: Vec<ScanFile>,
    pub edges: Vec<ScanEdge>,
    /// 依賴清單（package.json／Cargo.toml／requirements.txt 的套件名）。
    pub deps: Vec<String>,
    /// 檔案數超過上限，只掃了一部分。
    pub truncated: bool,
    pub git_head: Option<String>,
}
