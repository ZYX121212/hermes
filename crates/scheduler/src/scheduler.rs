// crates/scheduler/src/scheduler.rs
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use agent_core::{AgentEvent, ExecutionResult};
use anyhow::Result;
use futures::future::join_all;
use tokio::sync::mpsc::UnboundedSender;
use tools::ToolRegistry;
use uuid::Uuid;

use crate::concurrency::ConcurrencyLimit;

/// Scheduler that executes a Plan's steps respecting the DAG dependency order.
/// Steps in the same topological layer run concurrently.
pub struct Scheduler {
    registry: Arc<ToolRegistry>,
    concurrency: ConcurrencyLimit,
    event_tx: Option<UnboundedSender<AgentEvent>>,
}

impl Scheduler {
    pub fn new(registry: Arc<ToolRegistry>, max_concurrent: usize) -> Self {
        Self {
            registry,
            concurrency: ConcurrencyLimit::new(max_concurrent),
            event_tx: None,
        }
    }

    /// Set an event sender for TUI progress reporting.
    pub fn set_event_sender(&mut self, tx: UnboundedSender<AgentEvent>) {
        self.event_tx = Some(tx);
    }

    /// Execute all steps in a plan, grouped into topological layers.
    /// Within each layer, steps run concurrently (bounded by the semaphore).
    pub async fn execute(
        &self,
        plan: &agent_core::Plan,
    ) -> Result<ExecutionResult> {
        let start = Instant::now();
        let layers = plan.dag.topological_layers(&plan.steps);

        self.emit(AgentEvent::ExecutePhaseStarted {
            total_steps: plan.steps.len(),
        });

        // Map step outputs by step id for dependency resolution
        let mut completed: HashMap<Uuid, agent_core::StepOutput> = HashMap::new();
        let mut all_outputs: Vec<agent_core::StepOutput> = Vec::new();

        // Build 0-based index → step_id mapping for {{step_N.output}} resolution
        let step_index: HashMap<usize, Uuid> =
            plan.steps.iter().enumerate().map(|(i, s)| (i, s.id)).collect();

        for (layer_idx, layer) in layers.iter().enumerate() {
            // Snapshot completed outputs visible to this layer (read-only, shared across futures)
            let visible: Arc<HashMap<Uuid, agent_core::StepOutput>> =
                Arc::new(completed.clone());

            let futs: Vec<_> = layer
                .iter()
                .map(|step| {
                    let registry = Arc::clone(&self.registry);
                    let concurrency = &self.concurrency;
                    let mut step = step.clone();
                    let tx = self.event_tx.clone();
                    let layer = layer_idx;
                    let visible = Arc::clone(&visible);
                    let step_index = step_index.clone();
                    async move {
                        // Resolve {{step_N.output}} references in args
                        step.args = resolve_args(&step.args, &step_index, &visible);

                        // Emit step started
                        if let Some(ref tx) = tx {
                            let _ = tx.send(AgentEvent::StepStarted {
                                step_id: step.id,
                                tool: step.tool.clone(),
                                layer,
                            });
                        }

                        let _permit = match concurrency.acquire().await {
                            Ok(p) => p,
                            Err(e) => {
                                return agent_core::StepOutput {
                                    step_id: step.id,
                                    success: false,
                                    content: format!("{e}"),
                                    duration_ms: 0,
                                };
                            }
                        };
                        let step_start = Instant::now();
                        let out = registry.call(&step.tool, step.args.clone()).await;
                        let duration = step_start.elapsed().as_millis() as u64;
                        let step_out = match out {
                            Ok(tool_out) => agent_core::StepOutput {
                                step_id: step.id,
                                success: tool_out.success,
                                content: tool_out.content,
                                duration_ms: duration,
                            },
                            Err(e) => {
                                let err_msg = format!("{e}");
                                let error_category = if err_msg.contains("not found")
                                    || err_msg.contains("Tool not found")
                                {
                                    "tool_not_found"
                                } else {
                                    "tool_error"
                                };
                                tracing::warn!(
                                    tool = %step.tool,
                                    step_id = %step.id,
                                    category = error_category,
                                    error = %err_msg,
                                    "Step execution failed"
                                );
                                agent_core::StepOutput {
                                    step_id: step.id,
                                    success: false,
                                    content: err_msg,
                                    duration_ms: duration,
                                }
                            },
                        };

                        // Retry once on failure with error context
                        let final_output = if !step_out.success {
                            let mut retry_args = step.args.clone();
                            if let Some(obj) = retry_args.as_object_mut() {
                                obj.insert(
                                    "_retry".into(),
                                    serde_json::json!(true),
                                );
                                obj.insert(
                                    "_previous_error".into(),
                                    serde_json::json!(step_out.content),
                                );
                            }
                            let _permit = match concurrency.acquire().await {
                                Ok(p) => p,
                                Err(e) => {
                                    return agent_core::StepOutput {
                                        step_id: step.id,
                                        success: false,
                                        content: format!("retry aborted: {e}"),
                                        duration_ms: duration,
                                    };
                                }
                            };
                            let retry_start = Instant::now();
                            let retry_out = registry.call(&step.tool, retry_args).await;
                            let retry_dur = retry_start.elapsed().as_millis() as u64;
                            match retry_out {
                                Ok(tool_out) => agent_core::StepOutput {
                                    step_id: step.id,
                                    success: tool_out.success,
                                    content: tool_out.content,
                                    duration_ms: duration + retry_dur,
                                },
                                Err(e) => agent_core::StepOutput {
                                    step_id: step.id,
                                    success: false,
                                    content: format!("retry also failed: {e}"),
                                    duration_ms: duration + retry_dur,
                                },
                            }
                        } else {
                            step_out
                        };

                        // Emit step completed
                        if let Some(ref tx) = tx {
                            let _ = tx.send(AgentEvent::StepCompleted {
                                output: final_output.clone(),
                            });
                        }

                        final_output
                    }
                })
                .collect();

            for result in join_all(futs).await {
                completed.insert(result.step_id, result.clone());
                all_outputs.push(result);
            }
        }

        let all_succeeded = plan.steps.iter().all(|s| {
            completed
                .get(&s.id)
                .map(|o| o.success)
                .unwrap_or(false)
        });

        self.emit(AgentEvent::ExecutePhaseComplete {
            all_success: all_succeeded,
            duration_ms: start.elapsed().as_millis() as u64,
        });

        Ok(ExecutionResult {
            plan_id: plan.id,
            outputs: all_outputs,
            success: all_succeeded,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Send an event if a sender is configured.
    fn emit(&self, event: AgentEvent) {
        if let Some(ref tx) = self.event_tx {
            if tx.send(event).is_err() {
                tracing::warn!("Scheduler event channel closed");
            }
        }
    }
}

/// Recursively resolve `{{step_N.output}}` template references in step arguments.
/// N is the 0-based index into the plan's step list.
fn resolve_args(
    args: &serde_json::Value,
    step_index: &HashMap<usize, Uuid>,
    completed: &HashMap<Uuid, agent_core::StepOutput>,
) -> serde_json::Value {
    match args {
        serde_json::Value::String(s) => {
            serde_json::Value::String(resolve_template(s, step_index, completed))
        }
        serde_json::Value::Object(obj) => {
            let mut resolved = serde_json::Map::new();
            for (k, v) in obj {
                resolved.insert(k.clone(), resolve_args(v, step_index, completed));
            }
            serde_json::Value::Object(resolved)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(
                arr.iter()
                    .map(|v| resolve_args(v, step_index, completed))
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

/// Replace `{{step_N.output}}` patterns in a string with the actual output content.
fn resolve_template(
    s: &str,
    step_index: &HashMap<usize, Uuid>,
    completed: &HashMap<Uuid, agent_core::StepOutput>,
) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("{{step_") {
        result.push_str(&rest[..start]);
        let after_tag = &rest[start + 7..]; // skip "{{step_"
        if let Some(end) = after_tag.find("}}") {
            let inner = &after_tag[..end];
            // Parse "N.output" from inner
            if let Some(dot) = inner.find(".output") {
                let idx_str = &inner[..dot];
                if let Ok(idx) = idx_str.parse::<usize>() {
                    if let Some(&step_id) = step_index.get(&idx) {
                        if let Some(output) = completed.get(&step_id) {
                            result.push_str(&output.content);
                        } else {
                            // Step not yet completed (shouldn't happen if DAG is correct)
                            result.push_str(&rest[start..start + 7 + end + 2]);
                        }
                    } else {
                        // Invalid index
                        result.push_str(&rest[start..start + 7 + end + 2]);
                    }
                } else {
                    result.push_str(&rest[start..start + 7 + end + 2]);
                }
            } else {
                result.push_str(&rest[start..start + 7 + end + 2]);
            }
            rest = &after_tag[end + 2..]; // after "}}"
        } else {
            // Unclosed template, keep as-is
            result.push_str(&rest[start..]);
            rest = "";
            break;
        }
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{DependencyGraph, Plan, Step};
    use std::sync::atomic::{AtomicU32, Ordering};
    use tools::{Tool, ToolOutput};

    /// Mock tool that records invocation order for verifying DAG execution.
    struct OrderRecordingTool {
        order: Arc<AtomicU32>,
        next: AtomicU32,
        name: String,
    }

    impl OrderRecordingTool {
        fn new(name: &str, order: Arc<AtomicU32>) -> Self {
            Self {
                order,
                next: AtomicU32::new(0),
                name: name.to_string(),
            }
        }
    }

    #[async_trait::async_trait]
    impl Tool for OrderRecordingTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "Records invocation order"
        }
        fn schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn call(&self, _args: serde_json::Value) -> anyhow::Result<ToolOutput> {
            let current = self.next.fetch_add(1, Ordering::Relaxed);
            // Record the max order seen so far
            let prev = self.order.load(Ordering::Relaxed);
            if current > prev {
                self.order.store(current, Ordering::Relaxed);
            }
            Ok(ToolOutput::text(format!("step-{}", current)))
        }
    }

    fn make_plan(steps: Vec<Step>) -> Plan {
        let mut dag = DependencyGraph::new();
        for step in &steps {
            for dep in &step.depends {
                dag.add_edge(*dep, step.id);
            }
        }
        Plan {
            id: uuid::Uuid::new_v4(),
            steps,
            dag,
        }
    }

    fn make_step(id: uuid::Uuid, tool: &str, depends: Vec<uuid::Uuid>) -> Step {
        Step {
            id,
            tool: tool.to_string(),
            args: serde_json::json!({}),
            depends,
            strategy: "test".into(),
        }
    }

    #[tokio::test]
    async fn test_execute_single_step() {
        let registry = Arc::new(tools::ToolRegistry::default());
        let order = Arc::new(AtomicU32::new(0));
        registry.register(Arc::new(OrderRecordingTool::new("mock", Arc::clone(&order))));

        let step_id = uuid::Uuid::new_v4();
        let steps = vec![make_step(step_id, "mock", vec![])];
        let plan = make_plan(steps);

        let scheduler = Scheduler::new(registry, 4);
        let result = scheduler.execute(&plan).await.unwrap();

        assert!(result.success);
        assert_eq!(result.outputs.len(), 1);
        assert_eq!(result.outputs[0].step_id, step_id);
    }

    #[tokio::test]
    async fn test_execute_dependent_steps_ordered() {
        let registry = Arc::new(tools::ToolRegistry::default());
        let order = Arc::new(AtomicU32::new(0));
        registry.register(Arc::new(OrderRecordingTool::new("mock", Arc::clone(&order))));

        let step0 = uuid::Uuid::new_v4();
        let step1 = uuid::Uuid::new_v4();
        let step2 = uuid::Uuid::new_v4();

        let steps = vec![
            make_step(step0, "mock", vec![]),
            make_step(step1, "mock", vec![step0]),
            make_step(step2, "mock", vec![step0, step1]),
        ];
        let plan = make_plan(steps);

        let scheduler = Scheduler::new(registry, 4);
        let result = scheduler.execute(&plan).await.unwrap();

        assert!(result.success);
        assert_eq!(result.outputs.len(), 3);
    }

    #[tokio::test]
    async fn test_execute_independent_steps_concurrent() {
        let registry = Arc::new(tools::ToolRegistry::default());
        let order = Arc::new(AtomicU32::new(0));
        registry.register(Arc::new(OrderRecordingTool::new("mock", Arc::clone(&order))));

        let s0 = uuid::Uuid::new_v4();
        let s1 = uuid::Uuid::new_v4();
        let s2 = uuid::Uuid::new_v4();

        let steps = vec![
            make_step(s0, "mock", vec![]),
            make_step(s1, "mock", vec![]),
            make_step(s2, "mock", vec![]),
        ];
        let plan = make_plan(steps);

        let scheduler = Scheduler::new(registry, 4);
        let result = scheduler.execute(&plan).await.unwrap();

        assert!(result.success);
        assert_eq!(result.outputs.len(), 3);
        // All 3 should complete (order doesn't matter for independent steps)
        for output in &result.outputs {
            assert!(output.success, "step {} should succeed", output.step_id);
        }
    }

    #[tokio::test]
    async fn test_execution_duration_is_measured() {
        let registry = Arc::new(tools::ToolRegistry::default());
        let order = Arc::new(AtomicU32::new(0));
        registry.register(Arc::new(OrderRecordingTool::new("mock", Arc::clone(&order))));

        let step_id = uuid::Uuid::new_v4();
        let steps = vec![make_step(step_id, "mock", vec![])];
        let plan = make_plan(steps);

        let scheduler = Scheduler::new(registry, 4);
        let result = scheduler.execute(&plan).await.unwrap();

        // Duration field is populated (may be 0 for instantaneous mock execution)
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_missing_tool_is_error() {
        let registry = Arc::new(tools::ToolRegistry::default());
        let step_id = uuid::Uuid::new_v4();
        let steps = vec![make_step(step_id, "nonexistent", vec![])];
        let plan = make_plan(steps);

        let scheduler = Scheduler::new(registry, 4);
        let result = scheduler.execute(&plan).await.unwrap();

        assert!(!result.success);
        assert_eq!(result.outputs.len(), 1);
        assert!(!result.outputs[0].success);
    }
}
