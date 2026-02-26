use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentPhase {
    #[default]
    Explore,
    Think,
    Execute,
    Verify,
    Reflect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub number: i32,
    pub description: String,
    pub status: String,
}
