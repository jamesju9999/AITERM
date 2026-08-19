use std::path::{Path, PathBuf};
use std::fs;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use futures_util::stream::{self, StreamExt};

use crate::db::knowledge_base::{self, DocumentRow, NotebookRow};
use crate::knowledge_base::chunk::chunk_markdown;
use crate::knowledge_base::embedding::Embedder;

pub use crate::document_convert::{DocumentConverter, SUPPORTED_EXTENSIONS};

/// 單次同步最大併發轉換/embedding 數（見設計規格第 4 節安全限制）。
const MAX_CONCURRENT: usize = 3;

#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub mtime: i64,
}

/// 遞迴掃描資料夾，略過隱藏檔案/目錄，只保留支援格式的副檔名。
pub fn scan_folder(root: &Path) -> Vec<ScannedFile> {
    let mut out = Vec::new();
    scan_dir(root, root, &mut out);
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    out
}

fn scan_dir(dir: &Path, root: &Path, out: &mut Vec<ScannedFile>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            scan_dir(&path, root, out);
            continue;
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !SUPPORTED_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        let mtime = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let rel_path = path.strip_prefix(root).unwrap_or(&path)
            .to_string_lossy().replace('\\', "/");
        out.push(ScannedFile { rel_path, abs_path: path, mtime });
    }
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(h.finalize().iter().map(|b| format!("{:02x}", b)).collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncProgress {
    pub processed: usize,
    pub total: usize,
    pub current_file: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncSummary {
    pub indexed: usize,
    pub failed: usize,
    pub deleted: usize,
}

/// 對筆記本做一次增量同步：比對資料夾現況與既有 documents 紀錄，
/// 只轉換/切片/embedding 新增或內容變更的檔案（最多 MAX_CONCURRENT 個並行），
/// 刪除已消失的檔案紀錄。
pub async fn sync_notebook(
    pool: &SqlitePool,
    notebook: &NotebookRow,
    converter: Arc<dyn DocumentConverter>,
    embedder: Arc<dyn Embedder>,
    mut on_progress: impl FnMut(SyncProgress),
) -> Result<SyncSummary, String> {
    let root = Path::new(&notebook.folder_path);
    let scanned = scan_folder(root);
    let existing = knowledge_base::list_documents(pool, &notebook.id)
        .await.map_err(|e| e.to_string())?;

    let scanned_paths: HashSet<&str> = scanned.iter().map(|f| f.rel_path.as_str()).collect();
    let mut summary = SyncSummary::default();

    for doc in existing.iter().filter(|d| !scanned_paths.contains(d.rel_path.as_str())) {
        knowledge_base::delete_document_by_path(pool, &notebook.id, &doc.rel_path)
            .await.map_err(|e| e.to_string())?;
        summary.deleted += 1;
    }

    let existing_by_path: HashMap<&str, &DocumentRow> =
        existing.iter().map(|d| (d.rel_path.as_str(), d)).collect();

    // 先算 hash 判斷哪些檔案真的需要處理（新增或內容變更）。
    // 讀不到內容的檔案直接記成 error，不進入併發處理階段。
    let mut to_process: Vec<(ScannedFile, String)> = Vec::new();
    for file in &scanned {
        let hash = match hash_file(&file.abs_path) {
            Ok(h) => h,
            Err(e) => {
                knowledge_base::upsert_document(
                    pool, &notebook.id, &file.rel_path, file.mtime, "", None, "error", Some(&e),
                ).await.map_err(|e| e.to_string())?;
                summary.failed += 1;
                continue;
            }
        };
        let unchanged = existing_by_path.get(file.rel_path.as_str())
            .map(|d| d.content_hash == hash && d.status == "ok")
            .unwrap_or(false);
        if !unchanged {
            to_process.push((file.clone(), hash));
        }
    }

    let total = to_process.len();
    let mut processed = 0usize;

    let notebook_id = notebook.id.clone();
    let mut stream = stream::iter(to_process.into_iter().map(|(file, hash)| {
        spawn_process_one_file(
            pool.clone(),
            notebook_id.clone(),
            file.rel_path.clone(),
            file.abs_path.clone(),
            file.mtime,
            hash,
            converter.clone(),
            embedder.clone(),
        )
    })).buffer_unordered(MAX_CONCURRENT);

    while let Some(outcome) = stream.next().await {
        processed += 1;
        match outcome {
            Ok(rel_path) => {
                summary.indexed += 1;
                on_progress(SyncProgress { processed, total, current_file: rel_path });
            }
            Err((rel_path, _err)) => {
                summary.failed += 1;
                on_progress(SyncProgress { processed, total, current_file: rel_path });
            }
        }
    }

    Ok(summary)
}

/// Runs `process_one_file` on its own spawned task so a panic inside
/// `converter.convert()` or `embedder.embed()` — arbitrary trait-object
/// implementations; a real MarkItDown converter shells out to Python and
/// can panic on malformed subprocess output — is isolated to this one
/// file (surfaces as a `JoinError`) instead of unwinding through the
/// whole sync and losing every other file's progress.
async fn spawn_process_one_file(
    pool: SqlitePool,
    notebook_id: String,
    rel_path: String,
    abs_path: PathBuf,
    mtime: i64,
    hash: String,
    converter: Arc<dyn DocumentConverter>,
    embedder: Arc<dyn Embedder>,
) -> Result<String, (String, String)> {
    let rel_path_for_panic = rel_path.clone();
    let pool_for_panic = pool.clone();
    let notebook_id_for_panic = notebook_id.clone();
    let hash_for_panic = hash.clone();
    let handle = tokio::spawn(async move {
        process_one_file(&pool, &notebook_id, &rel_path, &abs_path, mtime, hash, converter.as_ref(), embedder.as_ref()).await
    });

    match handle.await {
        Ok(result) => result,
        Err(join_err) => {
            // The spawned task panicked (or was cancelled) before it could record its own
            // result, so `process_one_file`'s usual error-path upsert never ran. Write the
            // error document row here so the file still shows up as a tracked failure
            // instead of silently vanishing from the documents table.
            let err_msg = format!("processing task panicked or was cancelled: {join_err}");
            let _ = knowledge_base::upsert_document(
                &pool_for_panic, &notebook_id_for_panic, &rel_path_for_panic, mtime,
                &hash_for_panic, None, "error", Some(&err_msg),
            ).await;
            Err((rel_path_for_panic, err_msg))
        }
    }
}

/// 轉換單一檔案並寫入結果（成功或失敗都會 upsert 對應的 document row）。
/// 回傳 `Ok(rel_path)` 或 `Err((rel_path, error_message))`，方便併發收集結果時
/// 仍能標示是哪個檔案。
async fn process_one_file(
    pool: &SqlitePool,
    notebook_id: &str,
    rel_path: &str,
    abs_path: &Path,
    mtime: i64,
    hash: String,
    converter: &dyn DocumentConverter,
    embedder: &dyn Embedder,
) -> Result<String, (String, String)> {
    match process_one_file_inner(pool, notebook_id, rel_path, abs_path, mtime, &hash, converter, embedder).await {
        Ok(()) => Ok(rel_path.to_string()),
        Err(e) => {
            let _ = knowledge_base::upsert_document(
                pool, notebook_id, rel_path, mtime, &hash, None, "error", Some(&e),
            ).await;
            Err((rel_path.to_string(), e))
        }
    }
}

const CONVERT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// Must stay strictly less than `embedding::EMBED_TIMEOUT` (the HTTP client's own
/// timeout) — see the comment there for why. Applies per batch (see
/// `EMBED_BATCH_SIZE`), not to a whole file, so it stays meaningful regardless
/// of how many chunks a document has.
const EMBED_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// Max chunks sent in a single `embedder.embed()` call. A large source file
/// (multi-MB report, hundreds of chunks) used to be embedded in one HTTP
/// request; real local embedding servers choked on ~900-chunk requests well
/// past `EMBED_TIMEOUT`, so every chunk of an otherwise-fine document was lost
/// to a single oversized call. Splitting into bounded batches keeps each
/// request's cost roughly constant regardless of document size.
const EMBED_BATCH_SIZE: usize = 32;

async fn process_one_file_inner(
    pool: &SqlitePool,
    notebook_id: &str,
    rel_path: &str,
    abs_path: &Path,
    mtime: i64,
    hash: &str,
    converter: &dyn DocumentConverter,
    embedder: &dyn Embedder,
) -> Result<(), String> {
    let markdown = tokio::time::timeout(CONVERT_TIMEOUT, converter.convert(abs_path))
        .await
        .map_err(|_| format!("Document conversion timed out after {}s", CONVERT_TIMEOUT.as_secs()))??;
    let chunks = chunk_markdown(&markdown);

    // A converter can succeed (Ok) while producing no extractable text at all —
    // e.g. a scanned/image-only PDF with no OCR layer. Treating that as "ok" used
    // to silently index a document with zero chunks: it counted toward `indexed`,
    // showed no error anywhere, and was permanently unsearchable. Surface it as a
    // failure instead so it shows up in `summary.failed` and the document's
    // `error_message`, matching the "processing genuinely didn't work" contract
    // every other failure path in this function already follows.
    if chunks.is_empty() {
        return Err("轉換後沒有可用文字內容，可能是掃描或純圖片檔案，缺乏可擷取的文字".to_string());
    }

    let doc_id = knowledge_base::upsert_document(
        pool, notebook_id, rel_path, mtime, hash, Some(&markdown), "ok", None,
    ).await.map_err(|e| e.to_string())?;

    let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
    let mut embeddings: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
    for batch in texts.chunks(EMBED_BATCH_SIZE) {
        let batch_embeddings = tokio::time::timeout(EMBED_TIMEOUT, embedder.embed(batch))
            .await
            .map_err(|_| format!("Embedding request timed out after {}s", EMBED_TIMEOUT.as_secs()))??;
        if batch_embeddings.len() != batch.len() {
            return Err(format!(
                "Embedding count mismatch: {} chunks vs {} embeddings",
                batch.len(), batch_embeddings.len()
            ));
        }
        embeddings.extend(batch_embeddings);
    }

    let rows: Vec<(String, Option<String>, Vec<f32>)> = chunks.into_iter()
        .zip(embeddings)
        .map(|(c, e)| (c.text, c.location_hint, e))
        .collect();
    knowledge_base::replace_chunks(pool, &doc_id, &rows).await.map_err(|e| e.to_string())?;

    Ok(())
}
