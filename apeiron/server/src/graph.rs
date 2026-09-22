//! AP-G1..G4: the node graph model — kinds, ports, validation, topological
//! order, fan-out detection, and root selection.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub params: Value,
}

/// Port names stay camelCase on the wire for React Flow compatibility.
#[allow(non_snake_case)]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub source: String,
    #[serde(alias = "source_port", alias = "sourcePort")]
    pub sourcePort: String,
    pub target: String,
    #[serde(alias = "target_port", alias = "targetPort")]
    pub targetPort: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Graph {
    #[serde(default = "default_version")]
    pub version: i64,
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
}

fn default_version() -> i64 {
    1
}

impl Graph {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let graph: Graph =
            serde_json::from_str(raw).map_err(|e| format!("invalid graph JSON: {e}"))?;
        Ok(graph)
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn kind_of(&self, id: &str) -> Option<&str> {
        self.node(id).map(|node| node.kind.as_str())
    }
}

pub const NODE_KINDS: &[&str] = &[
    "note", "script", "storyboard", "image", "video", "tts", "subtitle", "material", "assemble",
];

/// Producer output port per kind; `None` for `note`.
pub fn output_port(kind: &str) -> Option<&'static str> {
    match kind {
        "script" => Some("text"),
        "storyboard" => Some("shots"),
        "image" => Some("image"),
        "video" => Some("video"),
        "tts" => Some("audio"),
        "subtitle" => Some("subtitle"),
        "material" => Some("video"),
        "assemble" => Some("video"),
        _ => None,
    }
}

/// Input port names per kind (AP-G2).
pub fn input_ports(kind: &str) -> &'static [&'static str] {
    match kind {
        "storyboard" => &["text"],
        "image" => &["text"],
        "video" => &["text", "image"],
        "tts" => &["text"],
        "subtitle" => &["text", "audio"],
        "material" => &["text"],
        "assemble" => &["clips", "audio", "subtitle"],
        _ => &[],
    }
}

/// AP-G3 port compatibility.
pub fn ports_compatible(target_port: &str, source_port: &str) -> bool {
    match target_port {
        "text" => matches!(source_port, "text" | "shots"),
        "clips" => matches!(source_port, "image" | "video"),
        "audio" => source_port == "audio",
        "image" => source_port == "image",
        "subtitle" => source_port == "subtitle",
        _ => false,
    }
}

/// Kinds whose per-shot steps fan out from a `shots` producer (AP-G2, AP-E2).
pub fn fan_out_kind(kind: &str) -> bool {
    matches!(kind, "image" | "video" | "material")
}

pub fn validate(graph: &Graph) -> Result<(), String> {
    let mut ids = HashSet::new();
    for node in &graph.nodes {
        if node.id.trim().is_empty() {
            return Err("node id must be non-empty".into());
        }
        if !ids.insert(node.id.clone()) {
            return Err(format!("duplicate node id {}", node.id));
        }
        if !NODE_KINDS.contains(&node.kind.as_str()) {
            return Err(format!("unknown node kind {}", node.kind));
        }
        if !node.params.is_object() {
            return Err(format!("node {} params must be an object", node.id));
        }
    }
    let mut edge_ids = HashSet::new();
    for edge in &graph.edges {
        if !edge_ids.insert(edge.id.clone()) {
            return Err(format!("duplicate edge id {}", edge.id));
        }
        let (source_kind, target_kind) = match (
            graph.kind_of(&edge.source),
            graph.kind_of(&edge.target),
        ) {
            (Some(s), Some(t)) => (s, t),
            _ => return Err(format!("edge {} references a missing node", edge.id)),
        };
        if source_kind == "note" {
            return Err(format!("edge {} starts at a note node", edge.id));
        }
        let Some(declared_source) = output_port(source_kind) else {
            return Err(format!("node kind {source_kind} produces no output"));
        };
        if edge.sourcePort != declared_source {
            return Err(format!(
                "edge {} source port {} does not match producer port {declared_source}",
                edge.id, edge.sourcePort
            ));
        }
        let ports = input_ports(target_kind);
        if !ports.contains(&edge.targetPort.as_str()) {
            return Err(format!(
                "node kind {target_kind} has no input port {}",
                edge.targetPort
            ));
        }
        if !ports_compatible(&edge.targetPort, declared_source) {
            return Err(format!(
                "edge {} connects incompatible ports {declared_source} -> {}",
                edge.id, edge.targetPort
            ));
        }
    }
    // Acyclicity (AP-G3).
    let mut indegree: HashMap<&str, usize> = HashMap::new();
    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    for node in &graph.nodes {
        indegree.entry(node.id.as_str()).or_insert(0);
    }
    for edge in &graph.edges {
        outgoing
            .entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
        *indegree.entry(edge.target.as_str()).or_default() += 1;
    }
    let mut queue: VecDeque<&str> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut visited = 0usize;
    while let Some(id) = queue.pop_front() {
        visited += 1;
        if let Some(next) = outgoing.get(id) {
            for target in next {
                let degree = indegree.get_mut(target).expect("node exists");
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(target);
                }
            }
        }
    }
    if visited != graph.nodes.len() {
        return Err("graph contains a cycle".into());
    }
    Ok(())
}

/// Nodes in topological order.
pub fn topo_order(graph: &Graph) -> Vec<&Node> {
    let mut indegree: HashMap<&str, usize> = HashMap::new();
    let mut incoming: HashMap<&str, Vec<&Edge>> = HashMap::new();
    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    for node in &graph.nodes {
        indegree.entry(node.id.as_str()).or_insert(0);
    }
    for edge in &graph.edges {
        outgoing
            .entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
        incoming
            .entry(edge.target.as_str())
            .or_default()
            .push(edge);
        *indegree.entry(edge.target.as_str()).or_default() += 1;
    }
    let mut queue: VecDeque<&str> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut order = Vec::with_capacity(graph.nodes.len());
    while let Some(id) = queue.pop_front() {
        if let Some(node) = graph.node(id) {
            order.push(node);
        }
        if let Some(next) = outgoing.get(id) {
            for target in next {
                let degree = indegree.get_mut(target).expect("node exists");
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(target);
                }
            }
        }
    }
    order.into_iter().filter(|node| node.kind != "note").collect()
}

/// Upstream node ids feeding `node_id` through any edge.
pub fn upstream_nodes(graph: &Graph, node_id: &str) -> Vec<String> {
    graph
        .edges
        .iter()
        .filter(|edge| edge.target == node_id)
        .map(|edge| edge.source.clone())
        .collect()
}

/// True when the node's `text` input is fed by a `storyboard` shots output.
pub fn fed_by_storyboard(graph: &Graph, node: &Node) -> bool {
    graph.edges.iter().any(|edge| {
        edge.target == node.id
            && edge.targetPort == "text"
            && graph.kind_of(&edge.source) == Some("storyboard")
    })
}

/// The storyboard node feeding `node`, if any.
pub fn storyboard_source(graph: &Graph, node: &Node) -> Option<String> {
    graph
        .edges
        .iter()
        .find(|edge| {
            edge.target == node.id
                && edge.targetPort == "text"
                && graph.kind_of(&edge.source) == Some("storyboard")
        })
        .map(|edge| edge.source.clone())
}

/// AP-G4: reachable node ids from the run root (assemble when present, else
/// every sink).
pub fn reachable_nodes(graph: &Graph, root: Option<&str>) -> Vec<String> {
    let mut targets: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &graph.edges {
        targets
            .entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
    }
    let roots: Vec<String> = match root {
        Some(id) if graph.node(id).is_some() => vec![id.to_string()],
        _ => {
            let mut sinks: Vec<String> = graph
                .nodes
                .iter()
                .filter(|node| {
                    node.kind != "note"
                        && !targets
                            .get(node.id.as_str())
                            .is_some_and(|list| !list.is_empty())
                })
                .map(|node| node.id.clone())
                .collect();
            if let Some(assemble) = graph
                .nodes
                .iter()
                .find(|node| node.kind == "assemble")
            {
                sinks = vec![assemble.id.clone()];
            }
            sinks
        }
    };
    // Reverse BFS from the roots to find every contributing node.
    let mut incoming: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &graph.edges {
        incoming
            .entry(edge.target.as_str())
            .or_default()
            .push(edge.source.as_str());
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = roots.into_iter().collect();
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(upstream) = incoming.get(id.as_str()) {
            for source in upstream {
                queue.push_back(source.to_string());
            }
        }
    }
    seen.into_iter().collect()
}
