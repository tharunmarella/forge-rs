use anyhow::Result;
use std::path::Path;
use tree_sitter::Parser;

/// Code symbol definition
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: usize,
    pub end_line: usize,
    pub signature: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SymbolKind {
    Function,
    Class,
    Struct,
    Enum,
    Interface,
    Type,
    Constant,
    Variable,
    Method,
    Module,
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Function => write!(f, "fn"),
            Self::Class => write!(f, "class"),
            Self::Struct => write!(f, "struct"),
            Self::Enum => write!(f, "enum"),
            Self::Interface => write!(f, "interface"),
            Self::Type => write!(f, "type"),
            Self::Constant => write!(f, "const"),
            Self::Variable => write!(f, "var"),
            Self::Method => write!(f, "method"),
            Self::Module => write!(f, "mod"),
        }
    }
}

/// Parse a file and extract symbol definitions
pub fn parse_definitions(content: &str, file_ext: &str) -> Result<Vec<Symbol>> {
    let mut parser = Parser::new();
    
    // Set language based on extension
    let language = match file_ext {
        "rs" => tree_sitter_rust::LANGUAGE,
        "py" => tree_sitter_python::LANGUAGE,
        "js" | "jsx" => tree_sitter_javascript::LANGUAGE,
        "ts" | "tsx" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        "go" => tree_sitter_go::LANGUAGE,
        _ => return Ok(Vec::new()),
    };
    
    parser.set_language(&language.into())?;
    
    let tree = parser.parse(content, None)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse"))?;
    
    let mut symbols = Vec::new();
    let root = tree.root_node();
    
    // Walk the tree and collect definitions
    collect_definitions(&root, content, file_ext, &mut symbols);
    
    Ok(symbols)
}

fn collect_definitions(
    node: &tree_sitter::Node,
    content: &str,
    file_ext: &str,
    symbols: &mut Vec<Symbol>,
) {
    let kind = node.kind();
    
    // Check if this is a definition node based on language
    let def_info = match file_ext {
        "rs" => match kind {
            "function_item" | "function_signature_item" => Some(("name", SymbolKind::Function)),
            "struct_item" => Some(("name", SymbolKind::Struct)),
            "enum_item" => Some(("name", SymbolKind::Enum)),
            "trait_item" => Some(("name", SymbolKind::Interface)),
            "impl_item" => Some(("type", SymbolKind::Method)),
            "type_item" => Some(("name", SymbolKind::Type)),
            "const_item" => Some(("name", SymbolKind::Constant)),
            "mod_item" => Some(("name", SymbolKind::Module)),
            _ => None,
        },
        "py" => match kind {
            "function_definition" => Some(("name", SymbolKind::Function)),
            "class_definition" => Some(("name", SymbolKind::Class)),
            _ => None,
        },
        "js" | "jsx" => match kind {
            "function_declaration" => Some(("name", SymbolKind::Function)),
            "class_declaration" => Some(("name", SymbolKind::Class)),
            "method_definition" => Some(("name", SymbolKind::Method)),
            _ => None,
        },
        "ts" | "tsx" => match kind {
            "function_declaration" => Some(("name", SymbolKind::Function)),
            "class_declaration" => Some(("name", SymbolKind::Class)),
            "interface_declaration" => Some(("name", SymbolKind::Interface)),
            "type_alias_declaration" => Some(("name", SymbolKind::Type)),
            "method_definition" => Some(("name", SymbolKind::Method)),
            _ => None,
        },
        "go" => match kind {
            "function_declaration" => Some(("name", SymbolKind::Function)),
            "method_declaration" => Some(("name", SymbolKind::Method)),
            "type_declaration" => Some(("name", SymbolKind::Type)),
            _ => None,
        },
        _ => None,
    };
    
    if let Some((name_field, sym_kind)) = def_info {
        // Try to find the name child
        if let Some(name_node) = node.child_by_field_name(name_field) {
            let name = name_node.utf8_text(content.as_bytes()).unwrap_or("").to_string();
            let start_line = node.start_position().row + 1;
            let end_line = node.end_position().row + 1;
            
            // Get first line as signature
            let start_byte = node.start_byte();
            let end_byte = (start_byte + 200).min(content.len());
            let sig = &content[start_byte..end_byte];
            let signature = sig.lines().next().unwrap_or("").to_string();
            
            if !name.is_empty() {
                symbols.push(Symbol {
                    name,
                    kind: sym_kind,
                    start_line,
                    end_line,
                    signature,
                });
            }
        }
    }
    
    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_definitions(&child, content, file_ext, symbols);
    }
}

/// Find a symbol definition by name
pub fn find_definition(content: &str, file_ext: &str, symbol_name: &str) -> Option<Symbol> {
    let symbols = parse_definitions(content, file_ext).ok()?;
    symbols.into_iter().find(|s| s.name == symbol_name)
}
