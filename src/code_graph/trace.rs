use petgraph::visit::EdgeRef;
use petgraph::Direction;
use crate::code_graph::{CodeGraph, SymbolNode, RelationType};
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct TraceResult {
    pub nodes: Vec<SymbolNode>,
    pub edges: Vec<(usize, usize, RelationType)>,
}

pub fn trace_calls(graph: &CodeGraph, symbol_name: &str, direction: &str, max_depth: usize) -> TraceResult {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut node_to_idx = std::collections::HashMap::new();

    // Find starting nodes
    let start_nodes: Vec<_> = graph.graph.node_indices()
        .filter(|&idx| graph.graph[idx].name == symbol_name)
        .collect();

    let mut queue = std::collections::VecDeque::new();
    for &node in &start_nodes {
        queue.push_back((node, 0));
    }

    while let Some((node_idx, depth)) = queue.pop_front() {
        if depth > max_depth || visited.contains(&node_idx) {
            continue;
        }
        visited.insert(node_idx);

        let node = &graph.graph[node_idx];
        let current_idx = nodes.len();
        nodes.push(node.clone());
        node_to_idx.insert(node_idx, current_idx);

        let dir = match direction {
            "upstream" => Direction::Incoming,
            "downstream" => Direction::Outgoing,
            _ => Direction::Outgoing, // Default
        };

        for edge in graph.graph.edges_directed(node_idx, dir) {
            let neighbor_idx = if dir == Direction::Outgoing { edge.target() } else { edge.source() };
            if !visited.contains(&neighbor_idx) && depth + 1 <= max_depth {
                queue.push_back((neighbor_idx, depth + 1));
            }
        }
    }

    // Add edges between collected nodes
    for &node_idx in &visited {
        for edge in graph.graph.edges(node_idx) {
            let source = edge.source();
            let target = edge.target();
            if visited.contains(&source) && visited.contains(&target) {
                edges.push((
                    *node_to_idx.get(&source).unwrap(),
                    *node_to_idx.get(&target).unwrap(),
                    edge.weight().clone()
                ));
            }
        }
    }

    TraceResult { nodes, edges }
}
