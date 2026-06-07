// crates/hermess-gateway/src/skills.rs
// Project-level skills influencing routing decisions.
//
// A .hermess/skills/ directory (similar to .claude/ or .github/) can be placed
// in any project directory. The gateway auto-discovers it on startup and merges
// skill-defined patterns, domain context, and route hints into the pipeline.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::models::RouteMode;

/// Format of a loaded skill file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SkillFormat {
    /// Legacy flat `.toml` file: `.hermess/skills/*.toml`
    #[default]
    Toml,
    /// Anthropic Agent Skills standard: `<name>/SKILL.md`
    SkillMd,
}

/// Internal struct for parsing SKILL.md YAML frontmatter.
#[derive(Debug, Clone, Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

/// A single project skill loaded from `.hermess/skills/`.
#[derive(Debug, Clone, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub route_hint: Option<RouteMode>,
    #[serde(default)]
    pub domain_tags: Vec<String>,
    #[serde(default)]
    pub shg_patterns: Vec<String>,
    /// Source format (TOML or SKILL.md). Not deserialized — set by loader.
    #[serde(skip)]
    #[allow(dead_code)]
    pub format: SkillFormat,
    /// Full Markdown body from SKILL.md files, for richer classifier context.
    #[serde(skip)]
    pub body_md: Option<String>,
}

/// Collection of skills discovered from a project directory.
#[derive(Debug, Clone, Default)]
pub struct SkillSet {
    pub skills: Vec<Skill>,
    pub source_dir: Option<PathBuf>,
}

impl SkillSet {
    /// Discover skills by walking up from the current directory.
    /// Returns the first `.hermess/skills/` found; empty SkillSet if none.
    pub fn discover() -> Self {
        let cwd = match std::env::current_dir() {
            Ok(d) => d,
            Err(_) => return Self::default(),
        };
        Self::discover_from(cwd)
    }

    /// Discover skills starting from `start_dir`, walking upward.
    pub fn discover_from(start_dir: PathBuf) -> Self {
        let mut current = start_dir.as_path();
        loop {
            let candidate = current.join(".hermess").join("skills");
            if candidate.is_dir() {
                tracing::info!(dir = %candidate.display(), "Discovered .hermess/skills directory");
                return Self::from_dir(&candidate);
            }
            match current.parent() {
                Some(parent) => current = parent,
                None => break,
            }
        }
        tracing::debug!("No .hermess/skills directory found");
        Self::default()
    }

    /// Load skills from a directory. Supports two formats:
    /// - `*.toml` — legacy flat TOML files
    /// - `<name>/SKILL.md` — Anthropic Agent Skills standard
    pub fn from_dir(dir: &Path) -> Self {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(dir = %dir.display(), error = %e, "Cannot read skills directory");
                return Self::default();
            }
        };

        let mut skills = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map(|e| e == "toml").unwrap_or(false) {
                // Legacy TOML format
                match std::fs::read_to_string(&path) {
                    Ok(content) => match toml::from_str::<Skill>(&content) {
                        Ok(skill) => {
                            tracing::debug!(name = %skill.name, path = %path.display(), "Loaded TOML skill");
                            skills.push(skill);
                        }
                        Err(e) => {
                            tracing::warn!(path = %path.display(), error = %e, "Skipping invalid skill file");
                        }
                    },
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "Skipping unreadable skill file");
                    }
                }
            } else if path.is_dir() {
                // Anthropic Agent Skills format: <name>/SKILL.md
                let skill_md = path.join("SKILL.md");
                if skill_md.is_file() {
                    match Self::from_skill_md(&skill_md) {
                        Ok(mut skill) => {
                            skill.format = SkillFormat::SkillMd;
                            tracing::debug!(name = %skill.name, path = %skill_md.display(), "Loaded SKILL.md");
                            skills.push(skill);
                        }
                        Err(e) => {
                            tracing::warn!(path = %skill_md.display(), error = %e, "Skipping invalid SKILL.md");
                        }
                    }
                }
            }
        }

        skills.sort_by(|a, b| a.name.cmp(&b.name));
        tracing::info!(count = skills.len(), dir = %dir.display(), "Loaded skills");

        Self {
            skills,
            source_dir: Some(dir.to_path_buf()),
        }
    }

    /// Parse a single Anthropic-style `SKILL.md` file.
    ///
    /// Extracts YAML frontmatter between `---` delimiters, maps standard
    /// `name`/`description` fields, and reads Hermess-specific routing config
    /// from `metadata.hermess.*` keys. The Markdown body is stored for
    /// richer classifier context.
    fn from_skill_md(path: &Path) -> Result<Skill, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;

        // Split YAML frontmatter from Markdown body
        let (frontmatter_str, body) = Self::split_frontmatter(&content)?;
        let fm: SkillFrontmatter = serde_yaml::from_str(&frontmatter_str)
            .map_err(|e| format!("Invalid YAML frontmatter: {e}"))?;

        // Extract Hermess-specific routing config from metadata
        let metadata = &fm.metadata;
        let route_hint = metadata
            .get("hermess.route_hint")
            .and_then(|v| v.parse().ok());
        let domain_tags: Vec<String> = metadata
            .get("hermess.domain_tags")
            .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();
        let shg_patterns: Vec<String> = metadata
            .get("hermess.shg_patterns")
            .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();

        let body_md = if body.trim().is_empty() {
            None
        } else {
            Some(body.trim().to_string())
        };

        Ok(Skill {
            name: fm.name,
            description: fm.description,
            route_hint,
            domain_tags,
            shg_patterns,
            format: SkillFormat::Toml, // caller overrides
            body_md,
        })
    }

    /// Split YAML frontmatter from Markdown body.
    /// Expects content starting with `---\n`, then frontmatter, then `\n---\n`.
    fn split_frontmatter(content: &str) -> Result<(String, &str), String> {
        let trimmed = content.trim_start();
        let after_first = trimmed
            .strip_prefix("---\n")
            .or_else(|| trimmed.strip_prefix("---\r\n"))
            .ok_or("Missing opening ---")?;
        let end = after_first
            .find("\n---")
            .or_else(|| after_first.find("\r\n---"))
            .ok_or("Missing closing ---")?;
        let frontmatter = &after_first[..end];
        // Skip "\n---\n" (5 bytes) or "\n---\r\n" (6 bytes) after frontmatter
        let delim_end = if after_first[end..].starts_with("\n---\r\n") {
            end + 6
        } else {
            end + 5 // "\n---\n"
        };
        let body = if delim_end < after_first.len() {
            &after_first[delim_end..]
        } else {
            ""
        };
        Ok((frontmatter.to_string(), body))
    }

    /// Collect deduplicated SHG patterns from all skills.
    pub fn shg_patterns(&self) -> Vec<String> {
        let mut patterns: Vec<String> = Vec::new();
        for skill in &self.skills {
            for pat in &skill.shg_patterns {
                if !patterns
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(pat))
                {
                    patterns.push(pat.clone());
                }
            }
        }
        patterns
    }

    /// Build domain context string for injection into the classifier prompt.
    /// Includes skill descriptions, tags, and Markdown body content (truncated).
    pub fn domain_context(&self) -> String {
        let mut parts: Vec<String> = Vec::new();

        // Header line: skill names with tags
        let mut header_lines = Vec::new();
        for skill in &self.skills {
            if !skill.description.is_empty() {
                let tags = if skill.domain_tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", skill.domain_tags.join(", "))
                };
                header_lines.push(format!("- {}{}: {}", skill.name, tags, skill.description));
            }
        }
        if !header_lines.is_empty() {
            parts.push(header_lines.join("\n"));
        }

        // Append Markdown bodies from SKILL.md skills for richer context
        for skill in &self.skills {
            if let Some(ref body) = skill.body_md {
                let truncated = Self::truncate_str(body, 1500);
                parts.push(format!("\n---\n## {}\n{truncated}", skill.name));
            }
        }

        parts.join("\n")
    }

    fn truncate_str(s: &str, max_chars: usize) -> &str {
        if s.len() <= max_chars {
            return s;
        }
        let cutoff = s
            .char_indices()
            .take(max_chars)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(max_chars);
        &s[..cutoff]
    }

    /// Build a system prompt that injects skill context into every LLM request.
    /// Empty string when no skills are loaded.
    pub fn system_prompt(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }

        let mut parts: Vec<String> = Vec::new();
        parts.push(
            "## Project Skills\n\nThe following skills describe this project's domain, \
             conventions, and constraints. Use them to guide your responses.\n"
                .to_string(),
        );

        for skill in &self.skills {
            let tags = if skill.domain_tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", skill.domain_tags.join(", "))
            };
            parts.push(format!("### {}{}", skill.name, tags));
            if !skill.description.is_empty() {
                parts.push(skill.description.clone());
            }
            if let Some(ref body) = skill.body_md {
                parts.push(format!("\n{body}"));
            }
            parts.push(String::new()); // blank line between skills
        }

        parts.join("\n")
    }

    /// Return the first skill-defined route hint, if any.
    pub fn route_hint(&self) -> Option<RouteMode> {
        for skill in &self.skills {
            if let Some(ref hint) = skill.route_hint {
                return Some(hint.clone());
            }
        }
        None
    }

    pub fn len(&self) -> usize {
        self.skills.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn names(&self) -> Vec<&str> {
        self.skills.iter().map(|s| s.name.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_skill(dir: &Path, name: &str, content: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!("{name}.toml"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    fn write_skill_md(dir: &Path, skill_name: &str, frontmatter: &str, body: &str) {
        let skill_dir = dir.join(skill_name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let content = format!("---\n{frontmatter}\n---\n{body}");
        std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn from_dir_loads_valid_skill() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        write_skill(
            &skills_dir,
            "rust",
            r#"
name = "rust-systems"
description = "Rust systems programming"
domain_tags = ["coding", "reasoning"]
shg_patterns = ["unsafe", "lifetime"]
"#,
        );

        let set = SkillSet::from_dir(&skills_dir);
        assert_eq!(set.len(), 1);
        assert_eq!(set.names(), vec!["rust-systems"]);
        assert_eq!(set.shg_patterns(), vec!["unsafe", "lifetime"]);
        assert!(set.domain_context().contains("rust-systems"));
        assert!(set.domain_context().contains("coding, reasoning"));
        assert!(set.route_hint().is_none());
    }

    #[test]
    fn from_dir_loads_skill_with_route_hint() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        write_skill(
            &skills_dir,
            "quality",
            r#"
name = "quality-project"
description = "Quality matters"
route_hint = "quality-first"
"#,
        );

        let set = SkillSet::from_dir(&skills_dir);
        assert_eq!(set.route_hint(), Some(RouteMode::QualityFirst));
    }

    #[test]
    fn from_dir_skips_invalid_toml() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("broken.toml"), "this is {{{ not toml").unwrap();

        let set = SkillSet::from_dir(&skills_dir);
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn from_dir_skips_non_toml_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("README.md"), "# Skills").unwrap();

        let set = SkillSet::from_dir(&skills_dir);
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn discover_from_walks_upward() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermess_dir = tmp.path().join(".hermess").join("skills");
        write_skill(
            &hermess_dir,
            "test",
            r#"
name = "deep-project"
description = "Found by walking up"
"#,
        );

        // Start from a subdirectory
        let deep = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&deep).unwrap();

        let set = SkillSet::discover_from(deep);
        assert_eq!(set.len(), 1);
        assert_eq!(set.names(), vec!["deep-project"]);
    }

    #[test]
    fn discover_returns_empty_when_none_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let set = SkillSet::discover_from(tmp.path().to_path_buf());
        assert!(set.skills.is_empty());
        assert!(set.shg_patterns().is_empty());
        assert!(set.domain_context().is_empty());
        assert!(set.route_hint().is_none());
    }

    #[test]
    fn shg_patterns_deduplicates() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        write_skill(
            &skills_dir,
            "a",
            r#"
name = "skill-a"
description = "A"
shg_patterns = ["unsafe", "concurrency"]
"#,
        );
        write_skill(
            &skills_dir,
            "b",
            r#"
name = "skill-b"
description = "B"
shg_patterns = ["Unsafe", "memory"]
"#,
        );

        let set = SkillSet::from_dir(&skills_dir);
        let patterns = set.shg_patterns();
        // "unsafe" and "Unsafe" should be deduplicated (case-insensitive)
        assert_eq!(patterns.len(), 3);
        assert!(patterns.iter().any(|p| p == "unsafe"));
        assert!(patterns.iter().any(|p| p == "concurrency"));
        assert!(patterns.iter().any(|p| p == "memory"));
    }

    #[test]
    fn domain_context_empty_when_no_description() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        write_skill(
            &skills_dir,
            "empty",
            r#"
name = "empty-skill"
description = ""
"#,
        );

        let set = SkillSet::from_dir(&skills_dir);
        assert!(set.domain_context().is_empty());
    }

    #[test]
    fn route_hint_returns_first_defined() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        write_skill(
            &skills_dir,
            "a",
            r#"
name = "first"
description = "First"
route_hint = "latency-first"
"#,
        );
        write_skill(
            &skills_dir,
            "b",
            r#"
name = "second"
description = "Second"
route_hint = "quality-first"
"#,
        );

        let set = SkillSet::from_dir(&skills_dir);
        assert_eq!(set.route_hint(), Some(RouteMode::LatencyFirst));
    }

    // --- SKILL.md (Anthropic Agent Skills format) tests ---

    #[test]
    fn from_skill_md_loads_basic() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        write_skill_md(
            &skills_dir,
            "rust-systems",
            "name: rust-systems\ndescription: Rust systems programming",
            "# Rust Guide\n\nUse `unsafe` carefully.",
        );

        let set = SkillSet::from_dir(&skills_dir);
        assert_eq!(set.len(), 1);
        assert_eq!(set.names(), vec!["rust-systems"]);
        let skill = &set.skills[0];
        assert_eq!(skill.format, SkillFormat::SkillMd);
        assert!(skill.body_md.as_ref().unwrap().contains("Rust Guide"));
    }

    #[test]
    fn from_skill_md_parses_metadata() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        write_skill_md(
            &skills_dir,
            "web-backend",
            concat!(
                "name: web-backend\n",
                "description: Web API development\n",
                "metadata:\n",
                "  hermess.route_hint: quality-first\n",
                "  hermess.domain_tags: api, web, database\n",
                "  hermess.shg_patterns: sql, auth, cors\n",
            ),
            "# Web Backend\n\nREST API best practices.",
        );

        let set = SkillSet::from_dir(&skills_dir);
        assert_eq!(set.len(), 1);
        assert_eq!(set.route_hint(), Some(RouteMode::QualityFirst));

        let skill = &set.skills[0];
        assert_eq!(skill.domain_tags, vec!["api", "web", "database"]);
        assert_eq!(skill.shg_patterns, vec!["sql", "auth", "cors"]);
        assert!(skill.body_md.as_ref().unwrap().contains("REST API"));
    }

    #[test]
    fn from_dir_mixed_toml_and_skill_md() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");

        // TOML skill
        write_skill(
            &skills_dir,
            "legacy",
            r#"
name = "legacy-skill"
description = "Old TOML skill"
shg_patterns = ["pattern-a"]
"#,
        );

        // SKILL.md skill
        write_skill_md(
            &skills_dir,
            "new-skill",
            "name: new-skill\ndescription: Anthropic-style skill\nmetadata:\n  hermess.shg_patterns: pattern-b",
            "# New skill body",
        );

        let set = SkillSet::from_dir(&skills_dir);
        assert_eq!(set.len(), 2);
        // Sorted by name: legacy-skill < new-skill
        assert_eq!(set.names(), vec!["legacy-skill", "new-skill"]);

        // SHG patterns merged from both
        let patterns = set.shg_patterns();
        assert!(patterns.iter().any(|p| p == "pattern-a"));
        assert!(patterns.iter().any(|p| p == "pattern-b"));
    }

    #[test]
    fn from_dir_skips_subdir_without_skill_md() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        // Create a subdirectory without SKILL.md — should be ignored
        std::fs::create_dir_all(skills_dir.join("empty-dir")).unwrap();
        // Create another file in a subdir that's not SKILL.md
        std::fs::create_dir_all(skills_dir.join("other-dir")).unwrap();
        std::fs::write(
            skills_dir.join("other-dir").join("README.md"),
            "# Not a skill",
        )
        .unwrap();

        let set = SkillSet::from_dir(&skills_dir);
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn domain_context_includes_body_md() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        write_skill_md(
            &skills_dir,
            "guide",
            "name: guide\ndescription: Coding guide",
            "# Coding Guide\n\n- Use consistent formatting\n- Write tests for all public APIs",
        );

        let set = SkillSet::from_dir(&skills_dir);
        let ctx = set.domain_context();
        // Header line present
        assert!(ctx.contains("- guide: Coding guide"));
        // Body appended
        assert!(ctx.contains("## guide"));
        assert!(ctx.contains("Use consistent formatting"));
    }

    #[test]
    fn domain_context_truncates_long_body() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        let long_body = "x".repeat(3000);
        write_skill_md(
            &skills_dir,
            "verbose",
            "name: verbose\ndescription: Very long body",
            &long_body,
        );

        let set = SkillSet::from_dir(&skills_dir);
        let ctx = set.domain_context();
        // Body should be truncated to 1500 chars
        let body_start = ctx.find("## verbose").unwrap();
        let body_part = &ctx[body_start..];
        // 1500 chars + "## verbose\n" overhead
        assert!(body_part.len() <= 1550);
    }

    #[test]
    fn from_skill_md_without_metadata() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        write_skill_md(
            &skills_dir,
            "simple",
            "name: simple\ndescription: No metadata skill",
            "",
        );

        let set = SkillSet::from_dir(&skills_dir);
        assert_eq!(set.len(), 1);
        let skill = &set.skills[0];
        assert!(skill.route_hint.is_none());
        assert!(skill.domain_tags.is_empty());
        assert!(skill.shg_patterns.is_empty());
        assert!(skill.body_md.is_none());
    }

    #[test]
    fn discover_from_walks_upward_for_skill_md() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermess_dir = tmp.path().join(".hermess").join("skills");
        write_skill_md(
            &hermess_dir,
            "upward",
            "name: upward-skill\ndescription: Found by walking up",
            "# Found!",
        );

        let deep = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&deep).unwrap();

        let set = SkillSet::discover_from(deep);
        assert_eq!(set.len(), 1);
        assert_eq!(set.names(), vec!["upward-skill"]);
    }

    #[test]
    fn split_frontmatter_basic() {
        let content = "---\nname: test\ndescription: A test skill\n---\n\n# Body text";
        let (fm, body) = SkillSet::split_frontmatter(content).unwrap();
        assert!(fm.contains("name: test"));
        assert!(body.contains("# Body text"));
    }

    #[test]
    fn split_frontmatter_missing_opening() {
        let content = "name: test\n---\nbody";
        assert!(SkillSet::split_frontmatter(content).is_err());
    }

    #[test]
    fn from_skill_md_invalid_yaml_skipped() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        let skill_dir = skills_dir.join("bad");
        std::fs::create_dir_all(&skill_dir).unwrap();
        // Invalid YAML (tab characters are not allowed in YAML indentation)
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: bad\n\tinvalid: yaml\n---\nbody",
        )
        .unwrap();

        let set = SkillSet::from_dir(&skills_dir);
        // Should be skipped, not crash
        assert_eq!(set.len(), 0);
    }

    // --- system_prompt tests ---

    #[test]
    fn system_prompt_empty_when_no_skills() {
        let set = SkillSet::default();
        assert!(set.system_prompt().is_empty());
    }

    #[test]
    fn system_prompt_includes_skill_info() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        write_skill(
            &skills_dir,
            "guide",
            r#"
name = "coding-guide"
description = "Write clean, tested code"
domain_tags = ["coding", "testing"]
"#,
        );

        let set = SkillSet::from_dir(&skills_dir);
        let prompt = set.system_prompt();
        assert!(prompt.contains("## Project Skills"));
        assert!(prompt.contains("### coding-guide [coding, testing]"));
        assert!(prompt.contains("Write clean, tested code"));
    }

    #[test]
    fn system_prompt_includes_markdown_body() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        write_skill_md(
            &skills_dir,
            "style",
            "name: style-guide\ndescription: Code style rules",
            "# Code Style\n\n- Use 4-space indent\n- Max line length 100",
        );

        let set = SkillSet::from_dir(&skills_dir);
        let prompt = set.system_prompt();
        assert!(prompt.contains("# Code Style"));
        assert!(prompt.contains("4-space indent"));
    }

    #[test]
    fn system_prompt_no_tags_when_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        write_skill(
            &skills_dir,
            "plain",
            r#"
name = "plain-skill"
description = "No tags here"
domain_tags = []
"#,
        );

        let set = SkillSet::from_dir(&skills_dir);
        let prompt = set.system_prompt();
        assert!(prompt.contains("### plain-skill"));
        // No bracket tags
        assert!(!prompt.contains("["));
    }
}
