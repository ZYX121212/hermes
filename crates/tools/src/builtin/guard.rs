// crates/tools/src/builtin/guard.rs
// 危险命令安全守卫：在 BashTool 执行前进行模式匹配和确认。

/// 确认策略
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationPolicy {
    /// 交互模式：弹窗/命令行确认
    Ask,
    /// 自动放行所有命令（不安全，仅用于受控环境）
    Skip,
    /// 自动拒绝所有危险命令
    Deny,
}

impl std::str::FromStr for ConfirmationPolicy {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "skip" => Self::Skip,
            "deny" => Self::Deny,
            _ => Self::Ask,
        })
    }
}

/// 危险命令守卫：检测高风险命令并请求确认。
pub struct DangerGuard {
    policy: ConfirmationPolicy,
    extra_patterns: Vec<String>,
}

impl DangerGuard {
    pub fn new(policy: ConfirmationPolicy, extra_patterns: Vec<String>) -> Self {
        Self { policy, extra_patterns }
    }

    /// 检查命令是否危险。返回 Ok(()) 表示可以继续，Err 表示被策略拒绝。
    pub fn check(&self, cmd: &str) -> Result<(), String> {
        if self.policy == ConfirmationPolicy::Skip {
            return Ok(());
        }

        if !self.is_dangerous(cmd) {
            return Ok(());
        }

        if self.policy == ConfirmationPolicy::Deny {
            return Err(format!(
                "危险命令已被安全策略自动拒绝: {}",
                Self::summarize(cmd)
            ));
        }

        // Ask 模式：返回危险信息由调用方处理确认
        Ok(())
    }

    /// 判断命令是否命中危险模式
    pub fn is_dangerous(&self, cmd: &str) -> bool {
        let cmd_lower = cmd.to_lowercase();
        let builtin = DANGEROUS_PATTERNS.iter().copied();
        let extra = self.extra_patterns.iter().map(|s| s.as_str());
        builtin.chain(extra).any(|pat| cmd_lower.contains(pat))
    }

    /// 返回危险命令的简短摘要（用于确认提示）
    pub fn summarize(cmd: &str) -> String {
        let truncated: String = cmd.chars().take(80).collect();
        if truncated.len() < cmd.len() {
            format!("{}…", truncated)
        } else {
            truncated
        }
    }

    /// 获取当前策略
    pub fn policy(&self) -> ConfirmationPolicy {
        self.policy
    }
}

/// 内置危险模式列表
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
    ".env",
    "eval \"$",
    "\\x", // 十六进制转义混淆
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detects_rm_rf() {
        let guard = DangerGuard::new(ConfirmationPolicy::Ask, vec![]);
        assert!(guard.is_dangerous("rm -rf /tmp/test"));
        assert!(guard.is_dangerous("rm -rf /"));
        assert!(!guard.is_dangerous("rm file.txt"));
        assert!(!guard.is_dangerous("echo hello"));
    }

    #[test]
    fn test_detects_sudo() {
        let guard = DangerGuard::new(ConfirmationPolicy::Ask, vec![]);
        assert!(guard.is_dangerous("sudo apt update"));
        assert!(guard.is_dangerous("sudo rm file"));
    }

    #[test]
    fn test_detects_git_force() {
        let guard = DangerGuard::new(ConfirmationPolicy::Ask, vec![]);
        assert!(guard.is_dangerous("git push --force origin main"));
        assert!(guard.is_dangerous("git push -f"));
        assert!(guard.is_dangerous("git reset --hard HEAD~5"));
    }

    #[test]
    fn test_detects_fork_bomb() {
        let guard = DangerGuard::new(ConfirmationPolicy::Ask, vec![]);
        assert!(guard.is_dangerous(":(){ :|:& };:"));
    }

    #[test]
    fn test_policy_skip() {
        let guard = DangerGuard::new(ConfirmationPolicy::Skip, vec![]);
        assert!(guard.check("rm -rf /").is_ok());
    }

    #[test]
    fn test_policy_deny() {
        let guard = DangerGuard::new(ConfirmationPolicy::Deny, vec![]);
        assert!(guard.check("rm -rf /").is_err());
    }

    #[test]
    fn test_extra_patterns() {
        let guard = DangerGuard::new(ConfirmationPolicy::Ask, vec!["my_custom_danger".into()]);
        assert!(guard.is_dangerous("my_custom_danger something"));
        assert!(!guard.is_dangerous("safe command"));
    }
}
