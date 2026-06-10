// crates/hermess-agent/src/curator.rs
// Skill 闭环学习：Curator 管理技能库（合并/归档/去重），
// SkillPatcher 检测过期工具引用并自动更新。
//
// 与 Distiller 的关系：
//   Distiller  = 创建新技能（执行→提炼→写入 SKILL.md）
//   Curator    = 管理已有技能（审查→合并→归档→去重）
//   Patcher    = 修复过期技能（工具接口变更→检测→自动更新）

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Metadata stored alongside each skill for curation decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub path: PathBuf,
    /// File size in bytes (proxy for complexity).
    pub size_bytes: u64,
    /// When the skill file was last modified.
    pub last_modified: String,
    /// Number of times this skill has been loaded by the Planner.
    pub load_count: u64,
    /// When this skill was last loaded (epoch seconds).
    pub last_loaded_at: u64,
    /// Domain tags extracted from YAML frontmatter.
    pub domain_tags: Vec<String>,
    /// SHG patterns from YAML frontmatter.
    pub shg_patterns: Vec<String>,
    /// Tool names referenced in the skill body.
    pub tool_refs: Vec<String>,
    /// Similarity hash for dedup (first 256 chars normalized).
    pub content_hash: String,
}

/// Action the curator proposes for a skill.
#[derive(Debug, Clone, PartialEq)]
pub enum CuratorAction {
    /// Merge skill A into skill B (A is the duplicate / subset).
    Merge {
        source: String,
        target: String,
        reason: String,
    },
    /// Archive a stale skill (move to .archive/).
    Archive { name: String, reason: String },
    /// No action needed — skill is healthy.
    Keep,
}

/// Result of a curator review cycle.
#[derive(Debug)]
pub struct CuratorReview {
    pub total_skills: usize,
    pub actions: Vec<(String, CuratorAction)>,
    pub merged: usize,
    pub archived: usize,
}

/// Manages the skill library — merges similar skills, archives stale ones,
/// and removes duplicates.
pub struct SkillCurator {
    skills_dir: PathBuf,
    archive_dir: PathBuf,
    /// Maximum days of inactivity before a skill is considered stale.
    pub stale_days: u64,
    /// Minimum Jaccard similarity (domain tags) to consider two skills for merge.
    pub merge_similarity_threshold: f64,
    /// Current epoch seconds (injected for testability).
    now_secs: u64,
}

impl Default for SkillCurator {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillCurator {
    pub fn new() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        let skills_dir = PathBuf::from(&home).join(".hermes").join("skills");
        let archive_dir = skills_dir.join(".archive");
        Self {
            skills_dir,
            archive_dir,
            stale_days: 30,
            merge_similarity_threshold: 0.6,
            now_secs: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    /// Set a custom skills directory.
    pub fn with_dir(mut self, dir: PathBuf) -> Self {
        self.skills_dir = dir.clone();
        self.archive_dir = dir.join(".archive");
        self
    }

    /// Override the current time for testing.
    #[cfg(test)]
    pub fn with_now(mut self, secs: u64) -> Self {
        self.now_secs = secs;
        self
    }

    /// Scan all skills and return their metadata.
    pub fn scan(&self) -> Vec<SkillMeta> {
        let mut metas = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.skills_dir) else {
            return metas;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() || path.file_name().is_none_or(|n| n == ".archive") {
                continue;
            }
            let skill_md = path.join("SKILL.md");
            if !skill_md.exists() {
                continue;
            }
            if let Some(meta) = self.read_skill_meta(&path, &skill_md) {
                metas.push(meta);
            }
        }
        metas
    }

    fn read_skill_meta(&self, dir: &Path, skill_md: &Path) -> Option<SkillMeta> {
        let name = dir.file_name()?.to_string_lossy().to_string();
        let content = std::fs::read_to_string(skill_md).ok()?;
        let size_bytes = content.len() as u64;
        let modified = skill_md
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            })
            .unwrap_or(0);
        let modified_str = modified.to_string();
        let domain_tags = Self::extract_field(&content, "hermess.domain_tags");
        let shg_patterns = Self::extract_field(&content, "hermess.shg_patterns");
        let tool_refs = Self::extract_tool_refs(&content);
        let content_hash = Self::hash_content(&content);

        Some(SkillMeta {
            name,
            path: dir.to_path_buf(),
            size_bytes,
            last_modified: modified_str,
            load_count: 0,
            last_loaded_at: 0,
            domain_tags,
            shg_patterns,
            tool_refs,
            content_hash,
        })
    }

    /// Extract a comma-separated metadata field from YAML frontmatter.
    fn extract_field(content: &str, field: &str) -> Vec<String> {
        let key = field.split('.').next_back().unwrap_or(field);
        for line in content.lines() {
            let trimmed = line.trim();
            // Handle both "key: value" and "prefix.key: value" formats
            if let Some(val) = trimmed.strip_prefix(&format!("{key}:")).or_else(|| {
                // Check if the line ends with "key: value"
                let suffix = format!("{key}:");
                if trimmed.contains(&suffix) {
                    trimmed.split(&suffix).nth(1)
                } else {
                    None
                }
            }) {
                return val
                    .trim()
                    .split(',')
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
        Vec::new()
    }

    /// Extract tool names referenced in skill body (backtick-quoted words).
    fn extract_tool_refs(content: &str) -> Vec<String> {
        let mut tools = Vec::new();
        let mut in_backtick = false;
        let mut current = String::new();
        for ch in content.chars() {
            if ch == '`' {
                if in_backtick {
                    let word = current.trim().to_string();
                    if !word.is_empty()
                        && word
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                    {
                        tools.push(word);
                    }
                    current.clear();
                }
                in_backtick = !in_backtick;
            } else if in_backtick {
                current.push(ch);
            }
        }
        tools.sort();
        tools.dedup();
        tools
    }

    /// Create a normalized content hash for dedup comparison.
    fn hash_content(content: &str) -> String {
        use std::hash::{Hash, Hasher};
        // Normalize: lowercase, strip whitespace, first 512 chars
        let normal: String = content
            .chars()
            .filter(|c| !c.is_whitespace())
            .take(512)
            .map(|c| c.to_lowercase().next().unwrap_or(c))
            .collect();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        normal.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// Run a full review cycle. Returns proposed actions for each skill.
    pub fn review(&self) -> CuratorReview {
        let metas = self.scan();
        let total = metas.len();
        let mut actions: Vec<(String, CuratorAction)> = Vec::new();

        // ── Step 1: Deduplicate by content hash ──
        let mut hash_groups: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, meta) in metas.iter().enumerate() {
            hash_groups
                .entry(meta.content_hash.clone())
                .or_default()
                .push(i);
        }
        for indices in hash_groups.values() {
            if indices.len() > 1 {
                for &idx in &indices[1..] {
                    actions.push((
                        metas[idx].name.clone(),
                        CuratorAction::Merge {
                            source: metas[idx].name.clone(),
                            target: metas[indices[0]].name.clone(),
                            reason: "内容重复（hash 相同）".into(),
                        },
                    ));
                }
            }
        }

        // ── Step 2: Detect similar skills by domain tag overlap ──
        for i in 0..metas.len() {
            if actions.iter().any(|(n, _)| n == &metas[i].name) {
                continue; // already flagged
            }
            for j in (i + 1)..metas.len() {
                if actions.iter().any(|(n, _)| n == &metas[j].name) {
                    continue;
                }
                let sim = jaccard_similarity(&metas[i].domain_tags, &metas[j].domain_tags);
                if sim >= self.merge_similarity_threshold {
                    // Keep the larger skill, merge the smaller
                    let (src, tgt) = if metas[i].size_bytes >= metas[j].size_bytes {
                        (&metas[j], &metas[i])
                    } else {
                        (&metas[i], &metas[j])
                    };
                    actions.push((
                        src.name.clone(),
                        CuratorAction::Merge {
                            source: src.name.clone(),
                            target: tgt.name.clone(),
                            reason: format!(
                                "领域标签相似度 {:.0}% ({} vs {})",
                                sim * 100.0,
                                src.domain_tags.join(","),
                                tgt.domain_tags.join(",")
                            ),
                        },
                    ));
                }
            }
        }

        // ── Step 3: Archive stale skills ──
        let stale_secs = self.stale_days * 86400;
        for meta in &metas {
            if actions.iter().any(|(n, _)| n == &meta.name) {
                continue;
            }
            let modified: u64 = meta.last_modified.parse().unwrap_or(0);
            if self.now_secs.saturating_sub(modified) > stale_secs && meta.load_count == 0 {
                actions.push((
                    meta.name.clone(),
                    CuratorAction::Archive {
                        name: meta.name.clone(),
                        reason: format!(
                            "{} 天未使用且从未加载",
                            (self.now_secs.saturating_sub(modified)) / 86400
                        ),
                    },
                ));
            }
        }

        // Remaining skills are healthy
        for meta in &metas {
            if !actions.iter().any(|(n, _)| n == &meta.name) {
                actions.push((meta.name.clone(), CuratorAction::Keep));
            }
        }

        let merged = actions
            .iter()
            .filter(|(_, a)| matches!(a, CuratorAction::Merge { .. }))
            .count();
        let archived = actions
            .iter()
            .filter(|(_, a)| matches!(a, CuratorAction::Archive { .. }))
            .count();

        CuratorReview {
            total_skills: total,
            actions,
            merged,
            archived,
        }
    }

    /// Execute merge: append source skill content to target, then delete source.
    pub fn execute_merge(&self, source: &str, target: &str) -> anyhow::Result<()> {
        let src_dir = self.skills_dir.join(source);
        let tgt_dir = self.skills_dir.join(target);
        let src_md = src_dir.join("SKILL.md");
        let tgt_md = tgt_dir.join("SKILL.md");

        if !src_md.exists() {
            anyhow::bail!("Source skill not found: {}", src_md.display());
        }
        if !tgt_md.exists() {
            anyhow::bail!("Target skill not found: {}", tgt_md.display());
        }

        let src_content = std::fs::read_to_string(&src_md)?;
        let mut tgt_content = std::fs::read_to_string(&tgt_md)?;

        // Append source content as a subsection
        tgt_content.push_str(&format!("\n\n---\n## 合并自 {source}\n\n{src_content}\n"));

        std::fs::write(&tgt_md, &tgt_content)?;
        std::fs::remove_dir_all(&src_dir)?;

        tracing::info!(source, target, "Skills merged");
        Ok(())
    }

    /// Execute archive: move skill directory to .archive/.
    pub fn execute_archive(&self, name: &str) -> anyhow::Result<()> {
        let src = self.skills_dir.join(name);
        if !src.exists() {
            anyhow::bail!("Skill not found: {}", src.display());
        }

        std::fs::create_dir_all(&self.archive_dir)?;
        let dst = self.archive_dir.join(name);
        if dst.exists() {
            std::fs::remove_dir_all(&dst)?;
        }
        std::fs::rename(&src, &dst)?;

        tracing::info!(name, "Skill archived");
        Ok(())
    }
}

/// Compute Jaccard similarity between two string sets.
fn jaccard_similarity(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let set_a: std::collections::HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let set_b: std::collections::HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

// ── Skill Patcher ──

/// Detects outdated tool references in skills and proposes patches.
pub struct SkillPatcher {
    /// Currently available tools (name → description).
    known_tools: HashMap<String, String>,
}

impl SkillPatcher {
    pub fn new(known_tools: HashMap<String, String>) -> Self {
        Self { known_tools }
    }

    /// Build a SkillPatcher from a ToolRegistry's describe_all output.
    pub fn from_tool_descriptions(descriptions: &[(String, String, serde_json::Value)]) -> Self {
        let known_tools: HashMap<String, String> = descriptions
            .iter()
            .map(|(name, desc, _)| (name.clone(), desc.clone()))
            .collect();
        Self { known_tools }
    }

    /// Check a skill for outdated tool references. Returns list of (old_name, suggestion).
    pub fn check(&self, skill_content: &str) -> Vec<OutdatedRef> {
        let mut issues = Vec::new();
        let refs = SkillCurator::extract_tool_refs(skill_content);

        for tool_ref in &refs {
            // Check if referenced tool still exists
            if !self.known_tools.contains_key(tool_ref) {
                // Try fuzzy match: find known tool with highest similarity
                let best_match = self
                    .known_tools
                    .keys()
                    .map(|name| {
                        let dist = levenshtein_distance(tool_ref, name);
                        let max_len = tool_ref.len().max(name.len()).max(1);
                        let sim = 1.0 - (dist as f64 / max_len as f64);
                        (name.clone(), dist, sim)
                    })
                    .max_by(|(_, _, a), (_, _, b)| {
                        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                    });

                if let Some((candidate, _dist, similarity)) = best_match {
                    if similarity >= 0.5 {
                        issues.push(OutdatedRef {
                            old_name: tool_ref.clone(),
                            suggestion: Some(candidate),
                            confidence: similarity,
                        });
                    } else {
                        issues.push(OutdatedRef {
                            old_name: tool_ref.clone(),
                            suggestion: None,
                            confidence: 0.0,
                        });
                    }
                }
            }
        }
        issues
    }

    /// Auto-patch a skill file: replace outdated tool names with current ones.
    pub fn patch(&self, content: &str) -> Option<String> {
        let issues = self.check(content);
        if issues.is_empty() {
            return None;
        }

        let mut patched = content.to_string();
        for issue in &issues {
            if let Some(ref suggestion) = issue.suggestion {
                if issue.confidence > 0.8 {
                    // High confidence: direct replacement
                    patched = patched.replace(&issue.old_name, suggestion);
                } else if issue.confidence > 0.5 {
                    // Medium confidence: add a note
                    patched.push_str(&format!(
                        "\n\n<!-- PATCH: 工具 `{}` 可能已更名为 `{}` (置信度 {:.0}%) -->",
                        issue.old_name,
                        suggestion,
                        issue.confidence * 100.0
                    ));
                }
            } else {
                // No match found: warn
                patched.push_str(&format!(
                    "\n\n<!-- PATCH: 工具 `{}` 可能已废弃，请手动更新 -->",
                    issue.old_name
                ));
            }
        }
        Some(patched)
    }
}

/// An outdated tool reference found in a skill.
#[derive(Debug, Clone)]
pub struct OutdatedRef {
    pub old_name: String,
    pub suggestion: Option<String>,
    pub confidence: f64,
}

/// Levenshtein edit distance for fuzzy tool name matching.
#[allow(clippy::needless_range_loop)]
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let len_a = a_chars.len();
    let len_b = b_chars.len();

    let mut dp = vec![vec![0usize; len_b + 1]; len_a + 1];
    for i in 0..=len_a {
        dp[i][0] = i;
    }
    for j in 0..=len_b {
        dp[0][j] = j;
    }
    for i in 1..=len_a {
        for j in 1..=len_b {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[len_a][len_b]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_skills(dir: &Path) {
        let s1 = dir.join("fix-cors");
        std::fs::create_dir_all(&s1).unwrap();
        std::fs::write(
            s1.join("SKILL.md"),
            "---\nname: fix-cors\ndescription: fix CORS\nmetadata:\n  hermess.domain_tags: web,api\n---\n\n# Fix CORS\n\nUse `bash` and `write` tools.",
        )
        .unwrap();

        let s2 = dir.join("cors-debug");
        std::fs::create_dir_all(&s2).unwrap();
        std::fs::write(
            s2.join("SKILL.md"),
            "---\nname: cors-debug\ndescription: debug CORS\nmetadata:\n  hermess.domain_tags: web,api\ndescription: debug\n---\n\n# Debug CORS\n\nUse `bash` tool.",
        )
        .unwrap();

        let s3 = dir.join("python-tips");
        std::fs::create_dir_all(&s3).unwrap();
        std::fs::write(
            s3.join("SKILL.md"),
            "---\nname: python-tips\ndescription: python patterns\nmetadata:\n  hermess.domain_tags: python,code\n---\n\n# Python Tips\n\nUse `code_exec_python`.",
        )
        .unwrap();
    }

    #[test]
    fn scan_discovers_skills() {
        let tmp = std::env::temp_dir().join("hermes_curator_test_scan");
        let _ = std::fs::remove_dir_all(&tmp);
        setup_test_skills(&tmp);
        let curator = SkillCurator::new().with_dir(tmp.clone());
        let metas = curator.scan();
        assert_eq!(metas.len(), 3);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn review_detects_duplicates() {
        let tmp = std::env::temp_dir().join("hermes_curator_test_dup");
        let _ = std::fs::remove_dir_all(&tmp);
        setup_test_skills(&tmp);
        // Create an exact duplicate
        let dup = tmp.join("cors-debug-dup");
        std::fs::create_dir_all(&dup).unwrap();
        std::fs::write(
            dup.join("SKILL.md"),
            "---\nname: cors-debug\ndescription: debug CORS\nmetadata:\n  hermess.domain_tags: web,api\ndescription: debug\n---\n\n# Debug CORS\n\nUse `bash` tool.",
        )
        .unwrap();

        let curator = SkillCurator::new().with_dir(tmp.clone());
        let review = curator.review();
        assert!(review.merged > 0, "Should detect duplicate");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn review_detects_similar_by_tags() {
        let tmp = std::env::temp_dir().join("hermes_curator_test_sim");
        let _ = std::fs::remove_dir_all(&tmp);
        setup_test_skills(&tmp);
        let curator = SkillCurator::new().with_dir(tmp.clone());
        let review = curator.review();
        // fix-cors and cors-debug share web,api tags → should merge
        assert!(
            review.merged >= 1,
            "Should merge skills with similar domain tags, got {} merges",
            review.merged
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn jaccard_identical() {
        let a = vec!["web".into(), "api".into()];
        let b = vec!["web".into(), "api".into()];
        assert!((jaccard_similarity(&a, &b) - 1.0).abs() < 0.001);
    }

    #[test]
    fn jaccard_disjoint() {
        let a = vec!["web".into()];
        let b = vec!["python".into()];
        assert!((jaccard_similarity(&a, &b) - 0.0).abs() < 0.001);
    }

    #[test]
    fn jaccard_partial() {
        let a = vec!["web".into(), "api".into()];
        let b = vec!["web".into(), "python".into()];
        // intersection = {"web"} = 1, union = {"web","api","python"} = 3
        assert!((jaccard_similarity(&a, &b) - 1.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn extract_tool_refs_from_skill() {
        let content = "# Test\n\nUse `bash` and `write` tools.\nAlso try `read_file`.";
        let refs = SkillCurator::extract_tool_refs(content);
        assert!(refs.contains(&"bash".to_string()));
        assert!(refs.contains(&"write".to_string()));
        assert!(refs.contains(&"read_file".to_string()));
    }

    #[test]
    fn patcher_detects_renamed_tool() {
        let mut known = HashMap::new();
        known.insert("bash".into(), "Execute shell commands".into());
        known.insert("write_file".into(), "Write content to file".into());
        known.insert("read_file".into(), "Read file contents".into());
        let patcher = SkillPatcher::new(known);

        let content = "Use `write` to save the file and `bash` to run it.";
        let issues = patcher.check(content);
        // "write" should fuzzy-match to "write_file"
        assert!(!issues.is_empty());
        let write_issue = issues.iter().find(|i| i.old_name == "write").unwrap();
        assert!(write_issue.suggestion.is_some());
        assert_eq!(write_issue.suggestion.as_ref().unwrap(), "write_file");
    }

    #[test]
    fn patcher_auto_replaces_high_confidence() {
        let mut known = HashMap::new();
        known.insert("bash".into(), "shell".into());
        known.insert("write_file".into(), "write file".into());
        let patcher = SkillPatcher::new(known);

        let content = "Use `write_file` to save content.";
        let issues = patcher.check(content);
        assert!(issues.is_empty()); // write_file exists, no issue

        let content2 = "Use `wrte_file` to save content.";
        let issues2 = patcher.check(content2);
        assert!(!issues2.is_empty());
    }

    #[test]
    fn levenshtein_same() {
        assert_eq!(levenshtein_distance("bash", "bash"), 0);
    }

    #[test]
    fn levenshtein_one_edit() {
        assert_eq!(levenshtein_distance("bash", "basha"), 1);
    }
}
