use crate::code_graph::{CodeGraph, SymbolNode};
use petgraph::Direction;
use petgraph::visit::EdgeRef;
use std::collections::{HashMap, HashSet};
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ImpactResult {
    pub symbol: String,
    pub total_affected: usize,
    pub files_affected: usize,
    pub by_file: HashMap<String, Vec<SymbolNode>>,
}

pub fn analyze_impact(graph: &CodeGraph, symbol_name: &str, max_depth: usize) -> ImpactResult {
    let mut affected_nodes = HashSet::new();
    let mut queue = std::collections::VecDeque::new();

    // Find starting nodes
    let start_nodes: Vec<_> = graph.graph.node_indices()
        .filter(|&idx| graph.graph[idx].name == symbol_name)
        .collect();

    for &node in &start_nodes {
        queue.push_back((node, 0));
    }

    while let Some((node_idx, depth)) = queue.pop_front() {
        if depth > max_depth || affected_nodes.contains(&node_idx) {
            continue;
        }
        affected_nodes.insert(node_idx);

        // Impact flows upstream (who depends on me?)
        for edge in graph.graph.edges_directed(node_idx, Direction::Incoming) {
            let neighbor_idx = edge.source();
            if !affected_nodes.contains(&neighbor_idx) && depth + 1 <= max_depth {
                queue.push_back((neighbor_idx, depth + 1));
            }
        }
    }

    let mut by_file: HashMap<String, Vec<SymbolNode>> = HashMap::new();
    for &idx in &affected_nodes {
        let node = &graph.graph[idx];
        by_file.entry(node.file_path.clone()).or_default().push(node.clone());
    }

    let files_affected = by_file.keys().count();
    let total_affected = affected_nodes.len();

    ImpactResult {
        symbol: symbol_name.to_string(),
        total_affected,
        files_affected,
        by_file,
    }
}
