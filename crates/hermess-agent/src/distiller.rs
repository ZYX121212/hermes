// crates/hermess-agent/src/distiller.rs
// Automatic skill distillation — converts task execution traces into
// reusable SKILL.md files after meeting trigger conditions.
//
// Triggers:
//  1. Tool calls > 5          — complex workflow worth capturing
//  2. Self-repair after error — learned how to fix a problem
//  3. User correction         — user steered the agent in a better direction
//  4. Non-obvious effective path — successful unusual tool combination

use std::path::PathBuf;

use agent_core::ExecutionResult;
use llm::LlmAdapter;

/// Detected trigger reason for skill creation.
#[derive(Debug, Clone, PartialEq)]
pub enum DistillTrigger {
    /// Many tool calls — complex workflow.
    ManyToolCalls { count: usize },
    /// Error occurred but was later fixed (self-healing).
    SelfRepair { failed_tools: Vec<String> },
    /// User corrected the agent's approach.
    UserCorrection,
    /// Non-obvious tool chain that worked well.
    EffectivePath { tools: Vec<String> },
}

impl std::fmt::Display for DistillTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DistillTrigger::ManyToolCalls { count } => write!(f, "工具调用 {count} 次"),
            DistillTrigger::SelfRepair { failed_tools } => {
                write!(f, "自修复: {}", failed_tools.join(", "))
            }
            DistillTrigger::UserCorrection => write!(f, "用户纠正"),
            DistillTrigger::EffectivePath { tools } => {
                write!(f, "有效路径: {}", tools.join(" → "))
            }
        }
    }
}

/// Result of attempting to distill a skill from an execution.
#[derive(Debug)]
pub enum DistillResult {
    /// No trigger met — nothing to distill.
    Skipped,
    /// Trigger met but LLM generation failed.
    Failed(String),
    /// Skill successfully written to disk.
    Written {
        name: String,
        path: PathBuf,
        trigger: DistillTrigger,
    },
}

/// Analyzes execution traces and distills reusable SKILL.md files.
pub struct SkillDistiller {
    output_dir: PathBuf,
    min_tool_calls: usize,
}

impl Default for SkillDistiller {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillDistiller {
    pub fn new() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        Self {
            output_dir: PathBuf::from(home).join(".hermes").join("skills"),
            min_tool_calls: 6,
        }
    }

    /// Set a custom output directory.
    pub fn with_output_dir(mut self, dir: PathBuf) -> Self {
        self.output_dir = dir;
        self
    }

    /// Check all trigger conditions against an execution trace.
    pub fn check_triggers(
        &self,
        result: &ExecutionResult,
        had_replan: bool,
        user_input: &str,
    ) -> Vec<DistillTrigger> {
        let mut triggers = Vec::new();

        // Trigger 1: Many tool calls (>= min_tool_calls)
        if result.outputs.len() >= self.min_tool_calls {
            triggers.push(DistillTrigger::ManyToolCalls {
                count: result.outputs.len(),
            });
        }

        // Trigger 2: Self-repair — any step failed AND we recovered (replan succeeded)
        let has_failure = result.outputs.iter().any(|o| !o.success);
        if has_failure && had_replan && result.success {
            let failed: Vec<String> = result
                .outputs
                .iter()
                .filter(|o| !o.success)
                .map(|o| o.tool.clone())
                .collect();
            triggers.push(DistillTrigger::SelfRepair {
                failed_tools: failed,
            });
        }

        // Trigger 3: User correction — simplified: check conversation context.
        // The caller passes had_user_correction flag based on conversation analysis.
        // We also check the user input for common correction patterns.
        if Self::has_correction_pattern(user_input) {
            triggers.push(DistillTrigger::UserCorrection);
        }

        // Trigger 4: Non-obvious effective path — succeeded with unusual tool combo
        if result.success && result.outputs.len() >= 3 {
            let tools: Vec<String> = result.outputs.iter().map(|o| o.tool.clone()).collect();
            let unique: Vec<String> = {
                let mut seen = std::collections::HashSet::new();
                tools
                    .into_iter()
                    .filter(|t| seen.insert(t.clone()))
                    .collect()
            };
            if unique.len() >= 3 {
                triggers.push(DistillTrigger::EffectivePath { tools: unique });
            }
        }

        triggers
    }

    /// Check if user input contains correction language.
    fn has_correction_pattern(input: &str) -> bool {
        let patterns = [
            "不对",
            "错了",
            "应该是",
            "改成",
            "不要",
            "不是这样",
            "换一种",
            "重新",
            "纠正",
            "修正",
            "不应该",
            "no",
            "wrong",
            "instead",
            "correct",
            "should be",
            "don't",
            "不是",
            "搞错了",
        ];
        let lower = input.to_lowercase();
        patterns.iter().any(|p| lower.contains(p))
    }

    /// Build the distillation prompt for the LLM.
    fn build_prompt(
        result: &ExecutionResult,
        triggers: &[DistillTrigger],
        user_input: &str,
    ) -> String {
        let tools_trace: Vec<String> = result
            .outputs
            .iter()
            .map(|o| {
                let status = if o.success { "✓" } else { "✗" };
                let preview = if o.content.len() > 200 {
                    let end = o
                        .content
                        .char_indices()
                        .take_while(|&(i, _)| i < 200)
                        .last()
                        .map(|(i, c)| i + c.len_utf8())
                        .unwrap_or(0);
                    format!("{}...", &o.content[..end])
                } else {
                    o.content.clone()
                };
                format!("  [{status}] {} ({}ms): {preview}", o.tool, o.duration_ms)
            })
            .collect();

        let trigger_desc: Vec<String> = triggers.iter().map(|t| format!("- {t}")).collect();

        format!(
            r#"你是一个技能蒸馏器。根据以下任务执行记录，提炼可复用的经验和流程，生成 SKILL.md 文件。

## 原始任务
{user_input}

## 触发原因
{}

## 执行过程
{}

## 要求
1. 提炼出一个**简洁的技能名称**（英文，kebab-case），如 `fix-cors-errors`
2. 写一段**中文描述**，说明此技能解决什么问题
3. 在 metadata 中标注合适的领域标签（hermess.domain_tags）
4. 如果涉及特定模式，在 hermess.shg_patterns 中标注
5. Body 部分用中文写清楚：问题场景、关键步骤、踩坑要点、正确做法

请按以下 YAML + Markdown 格式输出（不要输出其他内容）：

```yaml
---
name: <skill-name>
description: <一句话中文描述>
metadata:
  hermess.domain_tags: <逗号分隔>
  hermess.shg_patterns: <逗号分隔，可选>
---

# <技能标题>

## 适用场景
<什么情况下应该使用此技能>

## 关键步骤
1. ...
2. ...

## 易错点
- ...
- ...

## 验证方法
<如何确认操作正确>
```"#,
            trigger_desc.join("\n"),
            tools_trace.join("\n"),
        )
    }

    /// Attempt to distill a skill. Returns `Skipped` if no triggers met.
    pub async fn distill(
        &self,
        llm: &dyn LlmAdapter,
        result: &ExecutionResult,
        had_replan: bool,
        had_user_correction: bool,
        user_input: &str,
    ) -> DistillResult {
        let mut triggers = self.check_triggers(result, had_replan, user_input);

        // If the caller detected user correction from conversation but the
        // input text itself didn't match patterns, add it anyway.
        if had_user_correction
            && !triggers
                .iter()
                .any(|t| matches!(t, DistillTrigger::UserCorrection))
        {
            triggers.push(DistillTrigger::UserCorrection);
        }

        if triggers.is_empty() {
            return DistillResult::Skipped;
        }

        tracing::info!(
            triggers = ?triggers.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
            "Skill distillation triggered"
        );

        let prompt = Self::build_prompt(result, &triggers, user_input);

        let llm_req = llm::ChatCompletionRequest {
            messages: vec![llm::ChatMessage {
                role: "user".into(),
                content: prompt,
            }],
            max_tokens: Some(2048),
            temperature: Some(0.3),
            top_p: None,
            stream: false,
        };

        let response = match llm.complete_chat(llm_req).await {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!(error = %e, "Skill distillation LLM call failed");
                return DistillResult::Failed(format!("LLM call failed: {e}"));
            }
        };

        match parse_skill_md(&response) {
            Ok((name, content)) => {
                let dir = self.output_dir.join(&name);
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    return DistillResult::Failed(format!("mkdir {}: {e}", dir.display()));
                }
                let path = dir.join("SKILL.md");
                if let Err(e) = std::fs::write(&path, &content) {
                    return DistillResult::Failed(format!("write {}: {e}", path.display()));
                }
                tracing::info!(
                    name = %name,
                    path = %path.display(),
                    "Skill distilled and saved"
                );
                DistillResult::Written {
                    name,
                    path,
                    trigger: triggers.into_iter().next().unwrap(),
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, raw = %response, "Failed to parse distilled skill");
                DistillResult::Failed(format!("Parse error: {e}"))
            }
        }
    }
}

/// Parse the LLM output into (skill_name, full_content).
/// Expects YAML frontmatter with `name:` field.
fn parse_skill_md(raw: &str) -> Result<(String, String), String> {
    // Strip ```yaml and ``` fences if present
    let content = raw
        .trim()
        .strip_prefix("```yaml\n")
        .or_else(|| raw.trim().strip_prefix("```\n"))
        .unwrap_or(raw)
        .strip_suffix("\n```")
        .unwrap_or(raw.trim().strip_prefix("```yaml\n").unwrap_or(raw));

    // Extract name from YAML frontmatter
    let body = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
        .ok_or("Missing opening ---")?;

    let fm_end = body
        .find("\n---\n")
        .or_else(|| body.find("\r\n---\r\n"))
        .ok_or("Missing closing ---")?;
    let frontmatter = &body[..fm_end];

    let name = frontmatter
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            trimmed.strip_prefix("name:").map(|n| n.trim().to_string())
        })
        .ok_or("Missing name field in frontmatter")?;

    // Sanitize the name: kebab-case, no special chars
    let name = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_lowercase();

    if name.is_empty() || name.len() > 64 {
        return Err(format!("Invalid skill name: {name}"));
    }

    Ok((name, content.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(
        success: bool,
        outputs: Vec<(&str, bool, impl Into<String>)>,
    ) -> ExecutionResult {
        ExecutionResult {
            plan_id: uuid::Uuid::new_v4(),
            outputs: outputs
                .into_iter()
                .map(|(tool, ok, content)| agent_core::StepOutput {
                    step_id: uuid::Uuid::new_v4(),
                    tool: tool.to_string(),
                    success: ok,
                    content: content.into(),
                    duration_ms: 100,
                })
                .collect(),
            success,
            duration_ms: 500,
            user_input: Some("test task".into()),
        }
    }

    #[test]
    fn no_triggers_for_simple_task() {
        let d = SkillDistiller::new();
        let r = make_result(true, vec![("bash", true, "ok")]);
        let triggers = d.check_triggers(&r, false, "simple task");
        assert!(triggers.is_empty());
    }

    #[test]
    fn trigger_many_tool_calls() {
        let d = SkillDistiller::new();
        let outputs: Vec<_> = (0..7)
            .map(|i| ("bash", true, format!("step {i}")))
            .collect();
        let r = make_result(true, outputs);
        let triggers = d.check_triggers(&r, false, "complex task");
        assert!(triggers
            .iter()
            .any(|t| matches!(t, DistillTrigger::ManyToolCalls { .. })));
    }

    #[test]
    fn trigger_self_repair() {
        let d = SkillDistiller::new();
        let r = make_result(
            true,
            vec![
                ("bash", false, "command not found"),
                ("bash", true, "fixed with correct path"),
                ("write", true, "saved"),
            ],
        );
        let triggers = d.check_triggers(&r, true, "fix something");
        assert!(triggers
            .iter()
            .any(|t| matches!(t, DistillTrigger::SelfRepair { .. })));
    }

    #[test]
    fn trigger_user_correction() {
        let d = SkillDistiller::new();
        let r = make_result(true, vec![("write", true, "ok")]);
        let triggers = d.check_triggers(&r, false, "不对，应该用 Rust 而不是 Python");
        assert!(triggers
            .iter()
            .any(|t| matches!(t, DistillTrigger::UserCorrection)));
    }

    #[test]
    fn trigger_effective_path() {
        let d = SkillDistiller::new();
        let r = make_result(
            true,
            vec![
                ("bash", true, "git log"),
                ("grep", true, "found pattern"),
                ("write", true, "applied fix"),
            ],
        );
        let triggers = d.check_triggers(&r, false, "find and fix");
        assert!(triggers
            .iter()
            .any(|t| matches!(t, DistillTrigger::EffectivePath { .. })));
    }

    #[test]
    fn multiple_triggers() {
        let d = SkillDistiller::new();
        let outputs: Vec<_> = (0..8)
            .map(|i| ("bash", i != 2, format!("step {i}")))
            .collect();
        let r = make_result(true, outputs);
        let triggers = d.check_triggers(&r, true, "不对，应该换个方法");
        assert!(triggers.len() >= 3); // ManyToolCalls + SelfRepair + UserCorrection
    }

    #[test]
    fn parse_valid_skill_md() {
        let raw = r#"---
name: fix-cors-errors
description: 修复 CORS 跨域问题
metadata:
  hermess.domain_tags: web, api
---

# 修复 CORS 跨域问题

## 适用场景
前后端分离项目中遇到跨域错误时使用。
"#;
        let (name, content) = parse_skill_md(raw).unwrap();
        assert_eq!(name, "fix-cors-errors");
        assert!(content.contains("CORS"));
    }

    #[test]
    fn parse_strips_code_fences() {
        let raw = "```yaml\n---\nname: my-skill\ndescription: test\n---\n\n# Body\n```";
        let (name, _) = parse_skill_md(raw).unwrap();
        assert_eq!(name, "my-skill");
    }

    #[test]
    fn parse_sanitizes_name() {
        let raw = "---\nname: Fix CORS Errors!\ndescription: test\n---\n\n# Body";
        let (name, _) = parse_skill_md(raw).unwrap();
        assert_eq!(name, "fix-cors-errors");
    }

    #[test]
    fn parse_rejects_missing_name() {
        let raw = "---\ndescription: no name here\n---\n\n# Body";
        assert!(parse_skill_md(raw).is_err());
    }

    #[test]
    fn has_correction_pattern_detects_chinese() {
        assert!(SkillDistiller::has_correction_pattern("不对，你搞错了"));
        assert!(SkillDistiller::has_correction_pattern("应该是用 post 方法"));
        assert!(SkillDistiller::has_correction_pattern(
            "改成一个更简单的方案"
        ));
        assert!(!SkillDistiller::has_correction_pattern("帮我写一个函数"));
    }
}
