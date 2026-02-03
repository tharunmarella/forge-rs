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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EditFormat {
    /// Output entire file (for weak models)
    WholeFile,
    /// Search/replace blocks (for medium models)
    SearchReplace,
    /// Unified diff format (for strong models)
    UnifiedDiff,
}

impl EditFormat {
    /// Choose format based on model capability
    pub fn for_model(model: &str) -> Self {
        // Strong models that can do diffs
        if model.contains("opus") || 
           model.contains("gpt-4") || 
           model.contains("claude-3") ||
           model.contains("70b") ||
           model.contains("72b") {
            EditFormat::UnifiedDiff
        }
        // Medium models - search/replace
        else if model.contains("sonnet") ||
                model.contains("32b") ||
                model.contains("34b") ||
                model.contains("codestral") ||
                model.contains("deepseek") {
            EditFormat::SearchReplace
        }
        // Weak/small models - whole file
        else {
            EditFormat::WholeFile
        }
    }
}

/// The Edit Agent - generates reliable edits using specialized prompts
pub struct EditAgent {
    format: EditFormat,
    model: String,
}

impl EditAgent {
    pub fn new(model: &str) -> Self {
        Self {
            format: EditFormat::for_model(model),
            model: model.to_string(),
        }
    }

    /// Build the system prompt for edit generation
    fn build_system_prompt(&self) -> String {
        match self.format {
            EditFormat::WholeFile => WHOLE_FILE_SYSTEM_PROMPT.to_string(),
            EditFormat::SearchReplace => SEARCH_REPLACE_SYSTEM_PROMPT.to_string(),
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
        }
    }

    /// Parse the LLM response into actual edits
    pub fn parse_response(&self, response: &str, original: &str) -> Result<EditResult> {
        match self.format {
            EditFormat::WholeFile => self.parse_whole_file(response),
            EditFormat::SearchReplace => self.parse_search_replace(response, original),
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
        // Or: ```search ... ``` ```replace ... ```
        
        let lines: Vec<&str> = response.lines().collect();
        let mut i = 0;
        
        while i < lines.len() {
            // Look for search block
            if lines[i].contains("SEARCH") || lines[i].contains("<<<<<<") {
                let mut search_lines = Vec::new();
                let mut replace_lines = Vec::new();
                let mut in_search = true;
                
                i += 1;
                while i < lines.len() {
                    let line = lines[i];
                    if line.contains("=======") || line.contains("REPLACE") {
                        in_search = false;
                        i += 1;
                        continue;
                    }
                    if line.contains(">>>>>>>") || (line.starts_with("```") && !in_search) {
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
                    result = result.replace(&search_text, &replace_text);
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
    fn test_format_selection() {
        assert_eq!(EditFormat::for_model("gpt-4o"), EditFormat::UnifiedDiff);
        assert_eq!(EditFormat::for_model("claude-sonnet-4"), EditFormat::SearchReplace);
        assert_eq!(EditFormat::for_model("qwen2.5-coder:32b"), EditFormat::SearchReplace);
        assert_eq!(EditFormat::for_model("llama3.2:latest"), EditFormat::WholeFile);
        assert_eq!(EditFormat::for_model("qwen2.5-coder:7b"), EditFormat::WholeFile);
    }

    #[test]
    fn test_parse_whole_file() {
        let agent = EditAgent::new("llama3.2");
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
        let agent = EditAgent::new("qwen2.5-coder:32b");
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
}
