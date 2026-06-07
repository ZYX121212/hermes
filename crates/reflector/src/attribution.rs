// crates/reflector/src/attribution.rs
use agent_core::ExecutionResult;

/// Performs error attribution on a failed execution.
/// Identifies which steps failed and why, producing a human-readable lesson.
pub fn attribute_errors(result: &ExecutionResult) -> String {
    let failures: Vec<_> = result.outputs.iter().filter(|o| !o.success).collect();

    if failures.is_empty() {
        return format!(
            "All {} steps succeeded. Total duration: {}ms.",
            result.outputs.len(),
            result.duration_ms
        );
    }

    let mut summary = format!(
        "{} out of {} steps failed:\n",
        failures.len(),
        result.outputs.len()
    );
    for f in &failures {
        summary.push_str(&format!("  - step {}: {}\n", f.step_id, f.content));
    }
    summary.push_str(&format!(
        "Overall success: {}, Duration: {}ms",
        result.success, result.duration_ms
    ));
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::StepOutput;
    use uuid::Uuid;

    #[test]
    fn test_attribute_all_success() {
        let result = ExecutionResult {
            plan_id: Uuid::new_v4(),
            outputs: vec![StepOutput {
                step_id: Uuid::new_v4(),
                tool: "bash".into(),
                success: true,
                content: "ok".into(),
                duration_ms: 100,
            }],
            success: true,
            duration_ms: 100,
            user_input: None,
        };
        let s = attribute_errors(&result);
        assert!(s.contains("All"));
    }

    #[test]
    fn test_attribute_some_failures() {
        let result = ExecutionResult {
            plan_id: Uuid::new_v4(),
            outputs: vec![
                StepOutput {
                    step_id: Uuid::new_v4(),
                    tool: "bash".into(),
                    success: true,
                    content: "ok".into(),
                    duration_ms: 100,
                },
                StepOutput {
                    step_id: Uuid::new_v4(),
                    tool: "bash".into(),
                    success: false,
                    content: "command not found".into(),
                    duration_ms: 500,
                },
            ],
            success: false,
            duration_ms: 600,
            user_input: None,
        };
        let s = attribute_errors(&result);
        assert!(s.contains("1 out of 2"));
    }
}
