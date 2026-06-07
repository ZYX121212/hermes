// crates/memory/src/preload.rs
// Knowledge-base preloader: scans directories, chunks files, embeds text,
// and upserts into the vector memory store for RAG retrieval.
use std::path::Path;

use agent_core::MemoryStore;
use llm::LlmAdapter;

use crate::dedup::{self, DedupConfig};

/// Maximum file size to process (1 MB).
const MAX_FILE_SIZE: u64 = 1_024 * 1_024;
/// Maximum chunk size in characters.
const MAX_CHUNK_CHARS: usize = 2_000;
/// If more than this fraction of bytes in a file are non-UTF-8, skip it.
const BINARY_THRESHOLD: f64 = 0.1;

/// Scan a directory recursively, chunk text files, embed each chunk, and
/// upsert into the memory store. Reports progress via an optional callback.
pub async fn preload_knowledge_base(
    dir: &str,
    store: &dyn MemoryStore,
    embedder: &dyn LlmAdapter,
    on_progress: &(dyn Fn(String) + Send + Sync),
) -> anyhow::Result<KnowledgeBaseStats> {
    let root = Path::new(dir);
    if !root.exists() {
        tracing::info!(dir = %dir, "Knowledge base directory not found, skipping");
        return Ok(KnowledgeBaseStats::default());
    }

    let mut files: Vec<std::path::PathBuf> = Vec::new();
    collect_text_files(root, &mut files);

    let mut stats = KnowledgeBaseStats {
        files_found: files.len(),
        ..Default::default()
    };

    for file_path in &files {
        let rel = file_path.strip_prefix(root).unwrap_or(file_path);
        on_progress(format!("读取: {}", rel.display()));

        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %file_path.display(), error = %e, "无法读取文件");
                stats.files_skipped += 1;
                continue;
            }
        };

        if is_binary(content.as_bytes()) {
            stats.files_skipped += 1;
            continue;
        }

        let raw_chunks = chunk_text(&content, MAX_CHUNK_CHARS);
        stats.chunks_created += raw_chunks.len();

        let dedup_config = DedupConfig::default();
        let (chunks, dedup_stats) = dedup::deduplicate_chunks(raw_chunks, &dedup_config);
        stats.chunks_deduped += dedup_stats.removed_chunks;

        if dedup_stats.removed_chunks > 0 {
            tracing::debug!(
                file = %rel.display(),
                input = dedup_stats.input_chunks,
                kept = dedup_stats.kept_chunks,
                removed = dedup_stats.removed_chunks,
                "Chunk dedup"
            );
        }

        for chunk in chunks {
            on_progress(format!("嵌入: {} ({} 字符)", rel.display(), chunk.len()));

            let embedding = match embedder.embed(&chunk).await {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "嵌入失败，跳过 chunk");
                    stats.chunks_failed += 1;
                    continue;
                }
            };

            let memory_chunk = agent_core::MemoryChunk {
                id: uuid::Uuid::new_v4(),
                content: format!("[来源: {}] {}", rel.display(), chunk),
                embedding,
                timestamp: chrono::Utc::now(),
                importance: 1.0,
            };

            if let Err(e) = store.upsert(memory_chunk).await {
                tracing::warn!(error = %e, "Upsert 失败");
                stats.chunks_failed += 1;
            } else {
                stats.chunks_upserted += 1;
            }
        }
    }

    tracing::info!(
        files = stats.files_found,
        chunks = stats.chunks_upserted,
        skipped = stats.files_skipped,
        failed = stats.chunks_failed,
        "知识库预加载完成"
    );

    Ok(stats)
}

/// Preload statistics.
#[derive(Debug, Clone, Default)]
pub struct KnowledgeBaseStats {
    pub files_found: usize,
    pub files_skipped: usize,
    pub chunks_created: usize,
    pub chunks_deduped: usize,
    pub chunks_upserted: usize,
    pub chunks_failed: usize,
}

/// Recursively collect text files (by extension heuristic) from a directory.
fn collect_text_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'))
            {
                continue;
            }
            collect_text_files(&path, out);
        } else if path.is_file() && should_process(&path) {
            out.push(path);
        }
    }
}

/// Heuristic: only process files with text-like extensions or no extension.
fn should_process(path: &Path) -> bool {
    // Skip files that are too large
    if path
        .metadata()
        .map(|m| m.len() > MAX_FILE_SIZE)
        .unwrap_or(true)
    {
        return false;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "txt" | "md" | "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "c" | "cpp"
        | "h" | "hpp" | "rb" | "sh" | "bash" | "zsh" | "yaml" | "yml" | "toml" | "json" | "xml"
        | "html" | "css" | "scss" | "sql" | "r" | "lua" | "swift" | "kt" | "scala" | "clj"
        | "ex" | "exs" | "erl" | "hrl" | "hs" | "elm" | "vue" | "svelte" | "astro" | "csv"
        | "log" | "ini" | "cfg" | "conf" | "env" | "make" | "cmake" | "dockerfile"
        | "gitignore" => true,
        "" => true, // no extension, might be text
        _ => false,
    }
}

/// Detect if content is binary by checking the ratio of non-UTF-8 bytes.
fn is_binary(data: &[u8]) -> bool {
    if data.is_empty() {
        return true;
    }
    let non_utf8 = data
        .iter()
        .filter(|&&b| b == 0 || (0x80..0xC0).contains(&b) || b >= 0xF5)
        .count();
    (non_utf8 as f64 / data.len() as f64) > BINARY_THRESHOLD
}

/// Split text into overlapping chunks of at most `max_chars` characters.
fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    let mut chunks = Vec::new();
    let mut current = String::new();

    for para in paragraphs {
        let trimmed = para.trim();
        if trimmed.is_empty() {
            continue;
        }
        if current.len() + trimmed.len() + 2 > max_chars && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(trimmed);

        // If a single paragraph exceeds max_chars, split it further
        while current.len() > max_chars {
            // Find a sentence boundary near max_chars
            let split_at = find_split_point(&current, max_chars);
            let chunk = current[..split_at].trim().to_string();
            current = current[split_at..].trim().to_string();
            if !chunk.is_empty() {
                chunks.push(chunk);
            }
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

/// Find a good split point in text near `target`, preferring sentence boundaries.
fn find_split_point(text: &str, target: usize) -> usize {
    if target >= text.len() {
        return text.len();
    }
    let mut split_at = target;
    // Look for sentence-ending punctuation near the target
    for &pat in &['.', '!', '?', '\n', ';'] {
        if let Some(pos) = text[..target].rfind(pat) {
            split_at = pos + 1;
            break;
        }
    }
    // Fall back to the last space if no sentence boundary found
    if split_at == target {
        split_at = text[..target].rfind(' ').map(|p| p + 1).unwrap_or(target);
    }
    // Walk backward to ensure we're at a valid UTF-8 char boundary
    while split_at > 0 && !text.is_char_boundary(split_at) {
        split_at -= 1;
    }
    split_at
}
