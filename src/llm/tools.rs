use rig::tool::Tool;
use rig::completion::ToolDefinition;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::tools::{self, ToolCall};

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
}

impl Tool for ForgeToolAdapter {
    const NAME: &'static str = "forge_tool"; // This will be dynamic in definition
    type Error = ForgeToolError;
    type Args = serde_json::Value;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let call = ToolCall {
            name: self.name.clone(),
            arguments: args,
            thought_signature: None,
        };
        
        let workdir = self.workdir.clone();
        let plan_mode = self.plan_mode;

        // The issue is that tools::execute is not Send because it holds EmbeddingDb across awaits.
        // We need to ensure that no non-Send types are held across awaits in tools::execute.
        let result = tools::execute(&call, &workdir, plan_mode).await;

        if result.success {
            Ok(result.output)
        } else {
            Err(ForgeToolError(result.output))
        }
    }
}
