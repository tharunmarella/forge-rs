pub mod builder;
pub mod trace;
pub mod impact;

use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RelationType {
    Calls,
    Imports,
    BelongsTo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolNode {
    pub name: String,
    pub file_path: String,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
}

pub struct CodeGraph {
    pub graph: DiGraph<SymbolNode, RelationType>,
    pub symbol_map: HashMap<String, NodeIndex>,
}

impl CodeGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            symbol_map: HashMap::new(),
        }
    }

    pub fn add_symbol(&mut self, symbol: SymbolNode) -> NodeIndex {
        let key = format!("{}:{}", symbol.file_path, symbol.name);
        if let Some(&idx) = self.symbol_map.get(&key) {
            return idx;
        }
        let idx = self.graph.add_node(symbol);
        self.symbol_map.insert(key, idx);
        idx
    }

    pub fn add_relation(&mut self, from: NodeIndex, to: NodeIndex, rel: RelationType) {
        self.graph.add_edge(from, to, rel);
    }
}
