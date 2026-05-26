/// Combines responses from critical and regular model calls into one output.
pub struct ResultMerger;

impl ResultMerger {
    /// Merge two responses. Critical leads, regular follows.
    pub fn merge(critical: &str, regular: &str) -> String {
        if regular.is_empty() {
            return critical.to_string();
        }
        if critical.is_empty() {
            return regular.to_string();
        }
        format!("{critical}\n\n{regular}")
    }

    /// Merge with an explicit separator.
    #[allow(dead_code)]
    pub fn merge_with_separator(critical: &str, regular: &str, sep: &str) -> String {
        if regular.is_empty() {
            return critical.to_string();
        }
        if critical.is_empty() {
            return regular.to_string();
        }
        format!("{critical}{sep}{regular}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_both_present() {
        let result = ResultMerger::merge("Critical analysis", "Regular suggestions");
        assert_eq!(result, "Critical analysis\n\nRegular suggestions");
    }

    #[test]
    fn merge_empty_regular() {
        let result = ResultMerger::merge("Only critical", "");
        assert_eq!(result, "Only critical");
    }

    #[test]
    fn merge_empty_critical() {
        let result = ResultMerger::merge("", "Only regular");
        assert_eq!(result, "Only regular");
    }

    #[test]
    fn merge_with_separator() {
        let result = ResultMerger::merge_with_separator("A", "B", "\n---\n");
        assert_eq!(result, "A\n---\nB");
    }
}
