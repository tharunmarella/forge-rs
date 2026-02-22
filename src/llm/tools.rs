use rig::tool::Tool;
use rig::completion::ToolDefinition;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use crate::tools::{self, ToolCall};
use crate::api::{AgentPhase, PlanStep};

#[derive(Debug, Default)]
pub struct AgentState {
    pub current_phase: AgentPhase,
    pub plan: Vec<PlanStep>,
}

#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
#[error("Forge tool error: {0}")]
pub struct ForgeToolError(pub String);

#[derive(Deserialize, Serialize)]
pub struct ForgeToolAdapter {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub workdir: PathBuf,
    pub plan_mode: bool,
    #[serde(skip)]
    pub state: Arc<Mutex<AgentState>>,
}

impl Tool for ForgeToolAdapter {
    const NAME: &'static str = "forge_tool"; // This will be dynamic in definition
    type Error = ForgeToolError;
    type Args = serde_json::Value;
    type Output = String;

    // Override name() to return the dynamic name instead of the const NAME
    fn name(&self) -> String {
        self.name.clone()
    }

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let tool_name = self.name.clone();
        
        // Handle planning tools
        match tool_name.as_str() {
            "think" => {
                let mut state = self.state.lock().unwrap();
                state.current_phase = AgentPhase::Execute;
                return Ok(format!("Thinking complete. Transitioning to EXECUTE phase. Thought: {}", args.get("thought").and_then(|v| v.as_str()).unwrap_or("")));
            }
            "create_plan" => {
                let mut state = self.state.lock().unwrap();
                if let Some(steps) = args.get("steps").and_then(|v| v.as_array()) {
                    state.plan = steps.iter().enumerate().map(|(i, s)| PlanStep {
                        number: (i + 1) as i32,
                        description: s.as_str().unwrap_or("").to_string(),
                        status: "pending".to_string(),
                    }).collect();
                    state.current_phase = AgentPhase::Think;
                    return Ok(format!("Plan created with {} steps. Phase updated to THINK.", state.plan.len()));
                }
            }
            "update_plan" => {
                let mut state = self.state.lock().unwrap();
                let step_num = args.get("step_number").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let status = args.get("status").and_then(|v| v.as_str()).unwrap_or("");
                let desc = args.get("new_description").and_then(|v| v.as_str());

                if let Some(step) = state.plan.iter_mut().find(|s| s.number == step_num) {
                    step.status = status.to_string();
                    if let Some(d) = desc {
                        step.description = d.to_string();
                    }
                    return Ok(format!("Step {} updated to {}.", step_num, status));
                }
            }
            "add_plan_step" => {
                let mut state = self.state.lock().unwrap();
                let after = args.get("after_step").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                let desc = args.get("description").and_then(|v| v.as_str()).unwrap_or("");
                
                let new_step = PlanStep {
                    number: 0, // will be renumbered below
                    description: desc.to_string(),
                    status: "pending".to_string(),
                };
                
                // Insert AFTER the given step index (after=0 means prepend)
                let insert_at = after.min(state.plan.len());
                state.plan.insert(insert_at, new_step);
                
                // Renumber all steps sequentially
                for (i, step) in state.plan.iter_mut().enumerate() {
                    step.number = (i + 1) as i32;
                }
                return Ok(format!("Added new step at position {}.", insert_at + 1));
            }
            "remove_plan_step" => {
                let mut state = self.state.lock().unwrap();
                let step_num = args.get("step_number").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let before_len = state.plan.len();
                state.plan.retain(|s| s.number != step_num);
                // Renumber remaining steps
                for (i, step) in state.plan.iter_mut().enumerate() {
                    step.number = (i + 1) as i32;
                }
                if state.plan.len() < before_len {
                    return Ok(format!("Removed step {}. Plan now has {} steps.", step_num, state.plan.len()));
                } else {
                    return Ok(format!("Step {} not found in plan.", step_num));
                }
            }
            "discard_plan" => {
                let mut state = self.state.lock().unwrap();
                state.plan.clear();
                return Ok("Plan discarded.".to_string());
            }
            "replan" => {
                let mut state = self.state.lock().unwrap();
                let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("unspecified");
                if let Some(steps) = args.get("steps").and_then(|v| v.as_array()) {
                    state.plan = steps.iter().enumerate().map(|(i, s)| PlanStep {
                        number: (i + 1) as i32,
                        description: s.as_str().unwrap_or("").to_string(),
                        status: "pending".to_string(),
                    }).collect();
                    return Ok(format!("Replanned with {} steps. Reason: {}", state.plan.len(), reason));
                }
                return Ok(format!("Replan failed: missing 'steps'. Reason: {}", reason));
            }
            _ => {}
        }

        let call = ToolCall {
            name: tool_name.clone(),
            arguments: args,
            thought_signature: None,
        };
        
        let workdir = self.workdir.clone();
        let plan_mode = self.plan_mode;

        let result = tools::execute(&call, &workdir, plan_mode).await;

        if result.success {
            Ok(result.output)
        } else {
            Err(ForgeToolError(result.output))
        }
    }
}
