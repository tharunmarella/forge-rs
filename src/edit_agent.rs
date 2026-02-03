//! Edit Agent - Specialized LLM for generating reliable code edits
//!
//! Instead of asking the main agent to generate diffs directly,
//! we use a specialized prompt that's optimized for edit generation.
//! This dramatically improves edit reliability, especially for local models.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Edit request from the main agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditRequest {
    /// Path to the file being edited
    pub file_path: String,
    /// Current content of the file
    pub original_content: String,
    /// Description of what changes to make
    pub edit_description: String,
    /// Optional: specific lines to focus on
    pub focus_lines: Option<(usize, usize)>,
}

/// Edit result from the edit agent
#[derive(Debug, Clone)]
pub struct EditResult {
    /// The new content after edits
    pub new_content: String,
    /// Explanation of changes made
    pub explanation: String,
    /// Whether the edit was successful
    pub success: bool,
}

/// Edit format - different formats for different model capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EditFormat {
    /// Output entire file (safest, works with any model)
    WholeFile,
    /// Search/replace blocks (good balance of precision and reliability)
    SearchReplace,
    /// Unified diff format (most efficient, requires capable model)
    UnifiedDiff,
    /// Auto-detect based on model size hint in name
    Auto,
}

impl Default for EditFormat {
    fn default() -> Self {
        EditFormat::SearchReplace // Best default - works with most models
    }
}

impl EditFormat {
    /// Resolve Auto to a concrete format based on model name hints
    /// Users can override by setting explicit format in config
    pub fn resolve(self, model: &str) -> Self {
        if self != EditFormat::Auto {
            return self;
        }
        
        let model_lower = model.to_lowercase();
        
        // Look for size hints in model name
        // Large models (70B+) can do unified diff
        if model_lower.contains("70b") || 
           model_lower.contains("72b") ||
           model_lower.contains("405b") {
            EditFormat::UnifiedDiff
        }
        // Medium models (10B-40B) use search/replace
        else if model_lower.contains("14b") ||
                model_lower.contains("16b") ||
                model_lower.contains("22b") ||
                model_lower.contains("32b") ||
                model_lower.contains("34b") {
            EditFormat::SearchReplace
        }
        // Small models or unknown - safest is whole file
        else if model_lower.contains("7b") ||
                model_lower.contains("8b") ||
                model_lower.contains("3b") ||
                model_lower.contains("1b") {
            EditFormat::WholeFile
        }
        // Default to search/replace for unknown models
        else {
            EditFormat::SearchReplace
        }
    }
    
    /// Parse from string (for CLI args)
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "whole" | "whole-file" | "wholefile" => Some(EditFormat::WholeFile),
            "search-replace" | "searchreplace" | "sr" => Some(EditFormat::SearchReplace),
            "diff" | "unified-diff" | "udiff" => Some(EditFormat::UnifiedDiff),
            "auto" => Some(EditFormat::Auto),
            _ => None,
        }
    }
}

/// The Edit Agent - generates reliable edits using specialized prompts
pub struct EditAgent {
    format: EditFormat,
    model: String,
}

impl EditAgent {
    /// Create with auto-detected format
    pub fn new(model: &str) -> Self {
        Self::with_format(model, EditFormat::Auto)
    }
    
    /// Create with explicit format
    pub fn with_format(model: &str, format: EditFormat) -> Self {
        Self {
            format: format.resolve(model),
            model: model.to_string(),
        }
    }

    /// Build the system prompt for edit generation
    fn build_system_prompt(&self) -> String {
        match self.format {
            EditFormat::WholeFile => WHOLE_FILE_SYSTEM_PROMPT.to_string(),
            EditFormat::SearchReplace | EditFormat::Auto => SEARCH_REPLACE_SYSTEM_PROMPT.to_string(),
            EditFormat::UnifiedDiff => UNIFIED_DIFF_SYSTEM_PROMPT.to_string(),
        }
    }

    /// Build the user prompt for a specific edit
    pub fn build_edit_prompt(&self, request: &EditRequest) -> String {
        let focus_hint = if let Some((start, end)) = request.focus_lines {
            format!("\n\nFocus on lines {}-{}.", start, end)
        } else {
            String::new()
        };

        match self.format {
            EditFormat::WholeFile => format!(
                r#"# File: {}

## Current Content:
```
{}
```

## Edit Instructions:
{}{}

## Output:
Return the COMPLETE updated file content inside a code block."#,
                request.file_path,
                request.original_content,
                request.edit_description,
                focus_hint
            ),

            EditFormat::SearchReplace => format!(
                r#"# File: {}

## Current Content:
```
{}
```

## Edit Instructions:
{}{}

## Output:
Provide SEARCH/REPLACE blocks for each change needed."#,
                request.file_path,
                request.original_content,
                request.edit_description,
                focus_hint
            ),

            EditFormat::UnifiedDiff => format!(
                r#"# File: {}

## Current Content:
```
{}
```

## Edit Instructions:
{}{}

## Output:
Provide a unified diff (--- a/file, +++ b/file format) for the changes."#,
                request.file_path,
                request.original_content,
                request.edit_description,
                focus_hint
            ),
            
            EditFormat::Auto => {
                // Should be resolved before reaching here, fallback to SearchReplace
                self.build_edit_prompt_for_format(request, EditFormat::SearchReplace)
            }
        }
    }
    
    fn build_edit_prompt_for_format(&self, request: &EditRequest, format: EditFormat) -> String {
        let focus_hint = if let Some((start, end)) = request.focus_lines {
            format!("\n\nFocus on lines {}-{}.", start, end)
        } else {
            String::new()
        };
        
        match format {
            EditFormat::SearchReplace | EditFormat::Auto => format!(
                r#"# File: {}

## Current Content:
```
{}
```

## Edit Instructions:
{}{}

## Output:
Provide SEARCH/REPLACE blocks for each change needed."#,
                request.file_path,
                request.original_content,
                request.edit_description,
                focus_hint
            ),
            _ => self.build_edit_prompt(request),
        }
    }

    /// Parse the LLM response into actual edits
    pub fn parse_response(&self, response: &str, original: &str) -> Result<EditResult> {
        match self.format {
            EditFormat::WholeFile => self.parse_whole_file(response),
            EditFormat::SearchReplace | EditFormat::Auto => self.parse_search_replace(response, original),
            EditFormat::UnifiedDiff => self.parse_unified_diff(response, original),
        }
    }

    /// Parse whole-file response
    fn parse_whole_file(&self, response: &str) -> Result<EditResult> {
        // Extract content between code fences
        if let Some(start) = response.find("```") {
            let after_fence = &response[start + 3..];
            // Skip optional language tag
            let content_start = after_fence.find('\n').unwrap_or(0) + 1;
            let after_lang = &after_fence[content_start..];
            
            if let Some(end) = after_lang.find("```") {
                let content = after_lang[..end].to_string();
                return Ok(EditResult {
                    new_content: content,
                    explanation: "Replaced entire file".into(),
                    success: true,
                });
            }
        }
        
        Err(anyhow::anyhow!("Could not parse whole-file response"))
    }

    /// Parse search/replace blocks
    fn parse_search_replace(&self, response: &str, original: &str) -> Result<EditResult> {
        let mut result = original.to_string();
        let mut changes = 0;

        // Look for SEARCH/REPLACE patterns
        // Format: <<<<<<< SEARCH ... ======= ... >>>>>>> REPLACE
        
        let lines: Vec<&str> = response.lines().collect();
        let mut i = 0;
        
        while i < lines.len() {
            // Look for search block start
            if lines[i].contains("SEARCH") || lines[i].contains("<<<<<<") {
                let mut search_lines = Vec::new();
                let mut replace_lines = Vec::new();
                let mut in_search = true;
                
                i += 1;
                while i < lines.len() {
                    let line = lines[i];
                    
                    // Check for separator
                    if line.contains("=======") {
                        in_search = false;
                        i += 1;
                        continue;
                    }
                    
                    // Check for end marker
                    if line.contains(">>>>>>>") || line.contains("REPLACE") && line.contains(">") {
                        break;
                    }
                    
                    if in_search {
                        search_lines.push(line);
                    } else {
                        replace_lines.push(line);
                    }
                    i += 1;
                }
                
                let search_text = search_lines.join("\n");
                let replace_text = replace_lines.join("\n");
                
                if !search_text.is_empty() && result.contains(&search_text) {
                    result = result.replacen(&search_text, &replace_text, 1);
                    changes += 1;
                }
            }
            i += 1;
        }

        if changes > 0 {
            Ok(EditResult {
                new_content: result,
                explanation: format!("Applied {} search/replace block(s)", changes),
                success: true,
            })
        } else {
            // Fallback: try simple code block extraction
            self.parse_whole_file(response)
        }
    }

    /// Parse unified diff
    fn parse_unified_diff(&self, response: &str, original: &str) -> Result<EditResult> {
        // Try to apply the diff
        let mut result = original.to_string();
        let mut applied = false;

        // Simple line-by-line diff application
        let diff_lines: Vec<&str> = response.lines().collect();
        let mut additions = Vec::new();
        let mut deletions = Vec::new();

        for line in &diff_lines {
            if line.starts_with('+') && !line.starts_with("+++") {
                additions.push(&line[1..]);
            } else if line.starts_with('-') && !line.starts_with("---") {
                deletions.push(&line[1..]);
            }
        }

        // Apply deletions then additions
        for del in &deletions {
            if result.contains(*del) {
                result = result.replacen(*del, "", 1);
                applied = true;
            }
        }

        // For now, append additions if we can't find context
        // A more sophisticated implementation would use diff context
        if !additions.is_empty() && applied {
            // Try to find where to insert based on surrounding context
            // This is simplified - a real implementation would parse hunks
        }

        if applied {
            Ok(EditResult {
                new_content: result,
                explanation: format!("Applied diff: {} deletions, {} additions", deletions.len(), additions.len()),
                success: true,
            })
        } else {
            // Fallback to search/replace parsing
            self.parse_search_replace(response, original)
        }
    }

    /// Get the format being used
    pub fn format(&self) -> EditFormat {
        self.format
    }
}

// System prompts for different edit formats

const WHOLE_FILE_SYSTEM_PROMPT: &str = r#"You are a code editing assistant. Your task is to apply the requested changes to the given file.

IMPORTANT RULES:
1. Output the COMPLETE file content, not just the changed parts
2. Preserve all formatting, indentation, and style
3. Only make the changes described - do not add extra modifications
4. Wrap your output in a code block with the appropriate language

Example:
```rust
// entire file content here
```
"#;

const SEARCH_REPLACE_SYSTEM_PROMPT: &str = r#"You are a code editing assistant. Your task is to generate precise search/replace blocks.

Use this EXACT format for each change:

<<<<<<< SEARCH
exact text to find (copy precisely from the file)
=======
replacement text
>>>>>>> REPLACE

IMPORTANT RULES:
1. The SEARCH text must match EXACTLY - copy it from the file
2. Include enough context to make the match unique
3. You can have multiple SEARCH/REPLACE blocks
4. Preserve indentation exactly
"#;

const UNIFIED_DIFF_SYSTEM_PROMPT: &str = r#"You are a code editing assistant. Your task is to generate a unified diff.

Use this format:
```diff
--- a/filename
+++ b/filename
@@ -start,count +start,count @@
 context line
-deleted line
+added line
 context line
```

IMPORTANT RULES:
1. Include 3 lines of context before and after changes
2. Lines starting with - are removed
3. Lines starting with + are added
4. Lines starting with space are context (unchanged)
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_auto_resolution() {
        // Large models -> UnifiedDiff
        assert_eq!(EditFormat::Auto.resolve("llama-70b"), EditFormat::UnifiedDiff);
        
        // Medium models -> SearchReplace
        assert_eq!(EditFormat::Auto.resolve("qwen2.5-coder:32b"), EditFormat::SearchReplace);
        assert_eq!(EditFormat::Auto.resolve("model-14b"), EditFormat::SearchReplace);
        
        // Small models -> WholeFile
        assert_eq!(EditFormat::Auto.resolve("llama-7b"), EditFormat::WholeFile);
        assert_eq!(EditFormat::Auto.resolve("phi-3b"), EditFormat::WholeFile);
        
        // Unknown -> SearchReplace (safe default)
        assert_eq!(EditFormat::Auto.resolve("some-unknown-model"), EditFormat::SearchReplace);
        
        // Explicit format is preserved
        assert_eq!(EditFormat::WholeFile.resolve("llama-70b"), EditFormat::WholeFile);
    }
    
    #[test]
    fn test_format_from_str() {
        assert_eq!(EditFormat::from_str("whole-file"), Some(EditFormat::WholeFile));
        assert_eq!(EditFormat::from_str("search-replace"), Some(EditFormat::SearchReplace));
        assert_eq!(EditFormat::from_str("diff"), Some(EditFormat::UnifiedDiff));
        assert_eq!(EditFormat::from_str("auto"), Some(EditFormat::Auto));
        assert_eq!(EditFormat::from_str("invalid"), None);
    }

    #[test]
    fn test_parse_whole_file() {
        // Explicitly use WholeFile format
        let agent = EditAgent::with_format("any-model", EditFormat::WholeFile);
        let response = r#"Here's the updated file:

```rust
fn main() {
    println!("Hello, World!");
}
```
"#;
        let result = agent.parse_response(response, "").unwrap();
        assert!(result.success);
        assert!(result.new_content.contains("Hello, World!"));
    }

    #[test]
    fn test_parse_search_replace() {
        // Explicitly use SearchReplace format
        let agent = EditAgent::with_format("any-model", EditFormat::SearchReplace);
        let original = "fn main() {\n    println!(\"old\");\n}";
        let response = r#"
<<<<<<< SEARCH
    println!("old");
=======
    println!("new");
>>>>>>> REPLACE
"#;
        let result = agent.parse_response(response, original).unwrap();
        assert!(result.success);
        assert!(result.new_content.contains("new"));
        assert!(!result.new_content.contains("old"));
    }

    #[test]
    fn test_multiple_search_replace() {
        let agent = EditAgent::with_format("any-model", EditFormat::SearchReplace);
        let original = r#"fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn subtract(a: i32, b: i32) -> i32 {
    a - b
}"#;
        let response = r#"I'll update both functions:

<<<<<<< SEARCH
fn add(a: i32, b: i32) -> i32 {
=======
fn add(a: i64, b: i64) -> i64 {
>>>>>>> REPLACE

<<<<<<< SEARCH
fn subtract(a: i32, b: i32) -> i32 {
=======
fn subtract(a: i64, b: i64) -> i64 {
>>>>>>> REPLACE
"#;
        let result = agent.parse_response(response, original).unwrap();
        assert!(result.success);
        assert!(result.new_content.contains("i64"));
        assert!(!result.new_content.contains("i32"));
    }

    #[test]
    fn test_build_edit_prompt() {
        let agent = EditAgent::with_format("any-model", EditFormat::SearchReplace);
        let request = EditRequest {
            file_path: "src/main.rs".to_string(),
            original_content: "fn main() {}".to_string(),
            edit_description: "Add hello world print".to_string(),
            focus_lines: None,
        };
        
        let prompt = agent.build_edit_prompt(&request);
        assert!(prompt.contains("src/main.rs"));
        assert!(prompt.contains("fn main() {}"));
        assert!(prompt.contains("Add hello world print"));
        assert!(prompt.contains("SEARCH/REPLACE"));
    }

    #[test]
    fn test_whole_file_with_language_tag() {
        let agent = EditAgent::with_format("any-model", EditFormat::WholeFile);
        let response = r#"```python
def hello():
    print("Hello!")
```"#;
        let result = agent.parse_response(response, "").unwrap();
        assert!(result.success);
        assert!(result.new_content.contains("def hello()"));
    }

    #[test]
    fn test_real_world_edit() {
        // Simulate a real-world edit scenario with explicit format
        let agent = EditAgent::with_format("any-model", EditFormat::SearchReplace);
        
        let original = r#"use std::fs;

fn read_config() -> String {
    fs::read_to_string("config.txt").unwrap()
}

fn main() {
    let config = read_config();
    println!("{}", config);
}"#;

        // Simulate LLM response for "add error handling to read_config"
        let response = r#"I'll add proper error handling:

<<<<<<< SEARCH
fn read_config() -> String {
    fs::read_to_string("config.txt").unwrap()
}
=======
fn read_config() -> Result<String, std::io::Error> {
    fs::read_to_string("config.txt")
}
>>>>>>> REPLACE

<<<<<<< SEARCH
    let config = read_config();
=======
    let config = read_config().expect("Failed to read config");
>>>>>>> REPLACE
"#;

        let result = agent.parse_response(response, original).unwrap();
        assert!(result.success);
        assert!(result.new_content.contains("Result<String, std::io::Error>"));
        assert!(result.new_content.contains("expect(\"Failed to read config\")"));
        println!("=== Edited Content ===\n{}", result.new_content);
    }
}
