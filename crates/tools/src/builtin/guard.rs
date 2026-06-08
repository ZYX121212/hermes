// crates/tools/src/builtin/guard.rs
// 工具执行守卫系统：危险命令检测 + 三级审批 + 智能记忆。
use std::collections::HashMap;
use std::sync::Mutex;

/// 审批策略
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalPolicy {
    /// 自动放行（仅安全命令）
    Auto,
    /// 每次危险操作都询问
    Ask,
    /// 自动拒绝所有危险操作
    Deny,
}

impl std::str::FromStr for ApprovalPolicy {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "auto" | "skip" => Self::Auto,
            "deny" => Self::Deny,
            _ => Self::Ask,
        })
    }
}

/// 审批结果：调用方根据此结果决定是否允许执行。
#[derive(Debug, Clone)]
pub enum ApprovalResult {
    Allow,
    ConfirmRequired { danger_desc: String, cmd_summary: String },
    Denied { reason: String },
}

/// 用户对某类危险命令的历史决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserDecision {
    AllowOnce,
    DenyOnce,
    AllowAlways,
    DenyAlways,
}

/// 智能审批守卫：危险模式匹配 + 三级审批 + 用户决策记忆。
pub struct ToolGuard {
    policy: ApprovalPolicy,
    extra_patterns: Vec<String>,
    decisions: Mutex<HashMap<String, UserDecision>>,
}

impl ToolGuard {
    pub fn new(policy: ApprovalPolicy, extra_patterns: Vec<String>) -> Self {
        Self {
            policy,
            extra_patterns,
            decisions: Mutex::new(HashMap::new()),
        }
    }

    /// 检测命令并返回审批结果。
    pub fn approve(&self, cmd: &str) -> ApprovalResult {
        let dangerous = self.is_dangerous(cmd);
        if !dangerous {
            return ApprovalResult::Allow;
        }

        if let Some(decision) = self.lookup_decision(cmd) {
            return match decision {
                UserDecision::AllowOnce | UserDecision::AllowAlways => ApprovalResult::Allow,
                UserDecision::DenyOnce | UserDecision::DenyAlways => ApprovalResult::Denied {
                    reason: format!(
                        "denied by remembered decision for pattern: {}",
                        self.matched_pattern(cmd)
                    ),
                },
            };
        }

        match self.policy {
            ApprovalPolicy::Auto => ApprovalResult::Allow,
            ApprovalPolicy::Deny => ApprovalResult::Denied {
                reason: format!(
                    "危险命令被 Deny 策略自动拒绝: {}",
                    Self::summarize(cmd)
                ),
            },
            ApprovalPolicy::Ask => ApprovalResult::ConfirmRequired {
                danger_desc: format!(
                    "检测到危险命令模式: {}",
                    self.matched_pattern(cmd)
                ),
                cmd_summary: Self::summarize(cmd).to_string(),
            },
        }
    }

    /// 向后兼容的 check() 方法（DangerGuard 原有 API）。
    pub fn check(&self, cmd: &str) -> Result<(), String> {
        match self.approve(cmd) {
            ApprovalResult::Allow => Ok(()),
            ApprovalResult::ConfirmRequired { .. } => Ok(()),
            ApprovalResult::Denied { reason } => Err(reason),
        }
    }

    fn record_decision(&self, cmd: &str, decision: UserDecision) {
        let pattern = self.matched_pattern(cmd);
        if let Ok(mut decisions) = self.decisions.lock() {
            decisions.insert(pattern, decision);
        }
    }

    pub fn allow_once(&self, cmd: &str) {
        self.record_decision(cmd, UserDecision::AllowOnce);
    }

    pub fn allow_always(&self, cmd: &str) {
        self.record_decision(cmd, UserDecision::AllowAlways);
    }

    pub fn deny_once(&self, cmd: &str) {
        self.record_decision(cmd, UserDecision::DenyOnce);
    }

    pub fn deny_always(&self, cmd: &str) {
        self.record_decision(cmd, UserDecision::DenyAlways);
    }

    pub fn forget_all(&self) {
        if let Ok(mut decisions) = self.decisions.lock() {
            decisions.clear();
        }
    }

    pub fn is_dangerous(&self, cmd: &str) -> bool {
        let cmd_lower = cmd.to_lowercase();
        let builtin = DANGEROUS_PATTERNS.iter().copied();
        let extra = self.extra_patterns.iter().map(|s| s.as_str());
        builtin.chain(extra).any(|pat| cmd_lower.contains(pat))
    }

    pub fn matched_pattern(&self, cmd: &str) -> String {
        let cmd_lower = cmd.to_lowercase();
        let builtin = DANGEROUS_PATTERNS.iter().copied();
        let extra = self.extra_patterns.iter().map(|s| s.as_str());
        builtin
            .chain(extra)
            .find(|pat| cmd_lower.contains(*pat))
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".into())
    }

    fn lookup_decision(&self, cmd: &str) -> Option<UserDecision> {
        if let Ok(decisions) = self.decisions.lock() {
            decisions.get(&self.matched_pattern(cmd)).copied()
        } else {
            None
        }
    }

    pub fn summarize(cmd: &str) -> String {
        let truncated: String = cmd.chars().take(80).collect();
        if truncated.len() < cmd.len() {
            format!("{}...", truncated)
        } else {
            truncated
        }
    }

    pub fn policy(&self) -> ApprovalPolicy {
        self.policy
    }

    pub fn set_policy(&mut self, policy: ApprovalPolicy) {
        self.policy = policy;
    }
}

// 向后兼容别名
pub type ConfirmationPolicy = ApprovalPolicy;
pub type DangerGuard = ToolGuard;

/// 内置危险模式列表。
static DANGEROUS_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf ~",
    "rm -r /",
    "rm -rf /*",
    "rm -rf .git",
    "sudo rm",
    "sudo ",
    "chmod 777",
    "chmod -r 777",
    "git push --force",
    "git push -f",
    "git reset --hard",
    "git clean -fdx",
    "mkfs.",
    "dd if=",
    "/dev/sda",
    "/dev/nvme",
    ":(){ :|:& };:",
    "> /dev/sd",
    "curl .* | sh",
    "curl .* | bash",
    "wget .* | sh",
    "crontab -",
    "chown -R ",
    "docker rm -f",
    "kubectl delete",
    "shutdown",
    "reboot",
    "init 0",
    "init 6",
    "systemctl disable",
    "iptables -F",
    "ufw disable",
    "> /etc/",
    "/etc/passwd",
    "/etc/shadow",
    "~/.ssh/",
    ".env ",
    "eval \"$",
    "\\x",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detects_rm_rf() {
        let guard = ToolGuard::new(ApprovalPolicy::Ask, vec![]);
        assert!(guard.is_dangerous("rm -rf /tmp/test"));
        assert!(guard.is_dangerous("rm -rf /"));
        assert!(!guard.is_dangerous("rm file.txt"));
    }

    #[test]
    fn test_auto_policy_allows_all() {
        let guard = ToolGuard::new(ApprovalPolicy::Auto, vec![]);
        assert!(matches!(guard.approve("rm -rf /"), ApprovalResult::Allow));
    }

    #[test]
    fn test_deny_policy_blocks() {
        let guard = ToolGuard::new(ApprovalPolicy::Deny, vec![]);
        assert!(matches!(guard.approve("rm -rf /"), ApprovalResult::Denied { .. }));
    }

    #[test]
    fn test_ask_policy_requires_confirmation() {
        let guard = ToolGuard::new(ApprovalPolicy::Ask, vec![]);
        assert!(matches!(
            guard.approve("sudo rm important.txt"),
            ApprovalResult::ConfirmRequired { .. }
        ));
    }

    #[test]
    fn test_safe_command_allowed_in_ask() {
        let guard = ToolGuard::new(ApprovalPolicy::Ask, vec![]);
        assert!(matches!(guard.approve("ls -la"), ApprovalResult::Allow));
    }

    #[test]
    fn test_smart_approval_remembers_allow() {
        let guard = ToolGuard::new(ApprovalPolicy::Ask, vec![]);
        assert!(matches!(guard.approve("sudo systemctl restart"), ApprovalResult::ConfirmRequired { .. }));
        guard.allow_always("sudo systemctl restart");
        assert!(matches!(guard.approve("sudo apt update"), ApprovalResult::Allow));
    }

    #[test]
    fn test_smart_approval_remembers_deny() {
        let guard = ToolGuard::new(ApprovalPolicy::Ask, vec![]);
        guard.deny_always("rm -rf /tmp/data");
        assert!(matches!(guard.approve("rm -rf /tmp/data"), ApprovalResult::Denied { .. }));
    }

    #[test]
    fn test_forget_all() {
        let guard = ToolGuard::new(ApprovalPolicy::Ask, vec![]);
        guard.allow_always("sudo test");
        guard.forget_all();
        assert!(matches!(guard.approve("sudo test"), ApprovalResult::ConfirmRequired { .. }));
    }

    #[test]
    fn test_backward_compat() {
        // DangerGuard 是 ToolGuard 的别名
        let guard: DangerGuard = ToolGuard::new(ApprovalPolicy::Ask, vec![]);
        assert!(guard.is_dangerous("rm -rf /"));
        assert!(guard.check("echo hello").is_ok());
    }

    #[test]
    fn test_extra_patterns() {
        let guard = ToolGuard::new(ApprovalPolicy::Ask, vec!["my_custom_danger".into()]);
        assert!(guard.is_dangerous("my_custom_danger something"));
    }
}
