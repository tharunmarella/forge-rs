use super::ToolResult;
use serde_json::Value;
use std::time::Duration;

const CONTEXT7_API_BASE: &str = "https://context7.com/api/v1";

/// Fetch library documentation from Context7
pub async fn fetch_docs(args: &Value) -> ToolResult {
    let Some(library) = args.get("library").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'library' parameter");
    };
    
    let topic = args.get("topic").and_then(|v| v.as_str());
    
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default();
    
    // Step 1: Search for the library
    let search_query = if let Some(t) = topic {
        format!("{} {}", library, t)
    } else {
        library.to_string()
    };
    
    let search_url = format!(
        "{}/search?query={}",
        CONTEXT7_API_BASE,
        urlencoding::encode(&search_query)
    );
    
    let search_resp = match client
        .get(&search_url)
        .header("Accept", "application/json")
        .header("User-Agent", "forge-cli/1.0")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return ToolResult::err(format!("Search failed: {}", e)),
    };
    
    if !search_resp.status().is_success() {
        return ToolResult::err(format!("Context7 search failed: {}", search_resp.status()));
    }
    
    #[derive(serde::Deserialize)]
    struct SearchResult {
        id: String,
        title: String,
        #[serde(default)]
        description: String,
        score: f64,
    }
    
    #[derive(serde::Deserialize)]
    struct SearchResponse {
        #[serde(default)]
        results: Vec<SearchResult>,
    }
    
    let search_data: SearchResponse = match search_resp.json().await {
        Ok(d) => d,
        Err(e) => return ToolResult::err(format!("Failed to parse search results: {}", e)),
    };
    
    if search_data.results.is_empty() {
        return ToolResult::ok(format!("No documentation found for '{}'. Try a different library name or check spelling.", library));
    }
    
    // Get top result
    let top = &search_data.results[0];
    
    if top.score < 0.4 {
        return ToolResult::ok(format!(
            "No confident match for '{}'. Best guess: {} (score: {:.2})\nTry being more specific.",
            library, top.title, top.score
        ));
    }
    
    // Step 2: Fetch the documentation
    let doc_url = format!("{}/{}", CONTEXT7_API_BASE, top.id.trim_start_matches('/'));
    
    let doc_resp = match client
        .get(&doc_url)
        .header("Accept", "text/plain")
        .header("User-Agent", "forge-cli/1.0")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return ToolResult::err(format!("Failed to fetch docs: {}", e)),
    };
    
    if !doc_resp.status().is_success() {
        return ToolResult::err(format!("Failed to fetch documentation: {}", doc_resp.status()));
    }
    
    let mut content = match doc_resp.text().await {
        Ok(t) => t,
        Err(e) => return ToolResult::err(format!("Failed to read documentation: {}", e)),
    };
    
    // Truncate if too long
    const MAX_DOC_LENGTH: usize = 15000;
    if content.len() > MAX_DOC_LENGTH {
        content.truncate(MAX_DOC_LENGTH);
        content.push_str("\n\n... [Documentation truncated - use 'topic' parameter to focus on specific area]");
    }
    
    ToolResult::ok(format!(
        "# {} Documentation\n\nSource: {}\nRelevance: {:.0}%\n\n{}",
        top.title,
        top.id,
        top.score * 100.0,
        content
    ))
}

/// Web search (uses DuckDuckGo HTML)
pub async fn search(args: &Value) -> ToolResult {
    let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'query' parameter");
    };

    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding::encode(query)
    );

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; ForgeBot/1.0)")
        .build()
        .unwrap();

    match client.get(&url).send().await {
        Ok(resp) => {
            if let Ok(html) = resp.text().await {
                let results = parse_ddg_results(&html);
                if results.is_empty() {
                    ToolResult::ok("No results found")
                } else {
                    ToolResult::ok(results)
                }
            } else {
                ToolResult::err("Failed to read response")
            }
        }
        Err(e) => ToolResult::err(format!("Search failed: {e}")),
    }
}

/// Fetch URL content
pub async fn fetch(args: &Value) -> ToolResult {
    let Some(url) = args.get("url").and_then(|v| v.as_str()) else {
        return ToolResult::err("Missing 'url' parameter");
    };

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; ForgeBot/1.0)")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();

    match client.get(url).send().await {
        Ok(resp) => {
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("text/plain");

            if content_type.contains("text/html") {
                if let Ok(html) = resp.text().await {
                    let text = html_to_text(&html);
                    // Truncate if too long
                    let truncated = if text.len() > 10000 {
                        format!("{}...\n(truncated)", &text[..10000])
                    } else {
                        text
                    };
                    ToolResult::ok(truncated)
                } else {
                    ToolResult::err("Failed to read HTML")
                }
            } else if content_type.contains("application/json") {
                if let Ok(text) = resp.text().await {
                    ToolResult::ok(text)
                } else {
                    ToolResult::err("Failed to read JSON")
                }
            } else if content_type.contains("text/") {
                if let Ok(text) = resp.text().await {
                    ToolResult::ok(text)
                } else {
                    ToolResult::err("Failed to read text")
                }
            } else {
                ToolResult::err(format!("Unsupported content type: {content_type}"))
            }
        }
        Err(e) => ToolResult::err(format!("Fetch failed: {e}")),
    }
}

/// Parse DuckDuckGo HTML results
fn parse_ddg_results(html: &str) -> String {
    let mut results = Vec::new();
    
    // Simple regex-based extraction
    let title_re = regex::Regex::new(r#"class="result__a"[^>]*>([^<]+)</a>"#).unwrap();
    let url_re = regex::Regex::new(r#"class="result__url"[^>]*>([^<]+)</a>"#).unwrap();
    let snippet_re = regex::Regex::new(r#"class="result__snippet"[^>]*>([^<]+)"#).unwrap();

    let titles: Vec<&str> = title_re.captures_iter(html).filter_map(|c| c.get(1).map(|m| m.as_str())).collect();
    let urls: Vec<&str> = url_re.captures_iter(html).filter_map(|c| c.get(1).map(|m| m.as_str())).collect();
    let snippets: Vec<&str> = snippet_re.captures_iter(html).filter_map(|c| c.get(1).map(|m| m.as_str())).collect();

    for i in 0..titles.len().min(5) {
        let title = html_decode(titles.get(i).unwrap_or(&""));
        let url = urls.get(i).map(|u| u.trim()).unwrap_or("");
        let snippet = html_decode(snippets.get(i).unwrap_or(&""));
        
        results.push(format!("## {}\n{}\n{}\n", title, url, snippet));
    }

    results.join("\n")
}

/// Convert HTML to plain text (simple)
fn html_to_text(html: &str) -> String {
    let mut text = html.to_string();
    
    // Remove scripts and styles
    let script_re = regex::Regex::new(r"(?s)<script[^>]*>.*?</script>").unwrap();
    let style_re = regex::Regex::new(r"(?s)<style[^>]*>.*?</style>").unwrap();
    text = script_re.replace_all(&text, "").to_string();
    text = style_re.replace_all(&text, "").to_string();
    
    // Replace common tags
    text = text.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n");
    text = regex::Regex::new(r"</p>|</div>|</li>|</h[1-6]>").unwrap().replace_all(&text, "\n").to_string();
    
    // Remove all remaining tags
    let tag_re = regex::Regex::new(r"<[^>]+>").unwrap();
    text = tag_re.replace_all(&text, "").to_string();
    
    // Decode entities and clean up whitespace
    text = html_decode(&text);
    
    // Collapse whitespace
    let ws_re = regex::Regex::new(r"\n{3,}").unwrap();
    text = ws_re.replace_all(&text, "\n\n").to_string();
    
    text.trim().to_string()
}

/// Decode HTML entities
fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

mod urlencoding {
    pub fn encode(s: &str) -> String {
        s.chars()
            .map(|c| match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
                ' ' => "+".to_string(),
                _ => format!("%{:02X}", c as u8),
            })
            .collect()
    }
}
