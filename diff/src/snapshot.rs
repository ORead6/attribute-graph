use std::collections::BTreeMap;

use attribute_graph::{
    AttributeGraph, Edge, EdgeState, GraphError, NodeId, NodeKind, NodeState, SubgraphId,
    ValueComparison, ValueStorage,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphSnapshot {
    pub label: String,
    pub subgraphs: BTreeMap<SubgraphId, SubgraphSnapshot>,
    pub nodes: BTreeMap<NodeId, NodeSnapshot>,
    pub edges: BTreeMap<Edge, EdgeState>,
    pub pending_edges: Vec<Edge>,
}

impl GraphSnapshot {
    pub fn capture(label: impl Into<String>, graph: &AttributeGraph) -> Result<Self, GraphError> {
        Self::capture_with_labels(label, graph, &BTreeMap::new())
    }

    pub fn capture_with_labels(
        label: impl Into<String>,
        graph: &AttributeGraph,
        node_labels: &BTreeMap<NodeId, String>,
    ) -> Result<Self, GraphError> {
        Self::capture_with_label_maps(label, graph, node_labels, &BTreeMap::new())
    }

    pub fn capture_with_label_maps(
        label: impl Into<String>,
        graph: &AttributeGraph,
        node_labels: &BTreeMap<NodeId, String>,
        subgraph_labels: &BTreeMap<SubgraphId, String>,
    ) -> Result<Self, GraphError> {
        let mut subgraphs = BTreeMap::new();

        for id in graph.subgraphs() {
            let subgraph = graph.subgraph(id).ok_or(GraphError::MissingSubgraph(id))?;
            subgraphs.insert(
                id,
                SubgraphSnapshot {
                    id,
                    label: subgraph_labels.get(&id).cloned(),
                    parent: subgraph.parent(),
                    children: subgraph.children(),
                    nodes: subgraph.nodes(),
                },
            );
        }

        let mut nodes = BTreeMap::new();

        for id in graph.topological_order()? {
            let node = graph.node(id).ok_or(GraphError::MissingNode(id))?;
            let rule = node.rule();

            nodes.insert(
                id,
                NodeSnapshot {
                    id,
                    label: node_labels.get(&id).cloned(),
                    subgraph_id: node.subgraph_id(),
                    kind: node.kind(),
                    state: node.state(),
                    value_type: node
                        .value_type()
                        .map(|value_type| value_type.name().to_string()),
                    debug_name: rule.map(|rule| rule.debug_name().to_string()),
                    cached_value: graph.debug_cached_value(id).map(ValueSummary::from_storage),
                    dependencies: graph.dependencies_of(id)?,
                    dependents: graph.dependents_of(id)?,
                },
            );
        }

        let mut edges = BTreeMap::new();
        for edge in graph.edges() {
            edges.insert(edge, graph.edge_state(edge.dependency, edge.dependent)?);
        }

        Ok(Self {
            label: label.into(),
            subgraphs,
            nodes,
            edges,
            pending_edges: graph.pending_edges(),
        })
    }

    pub fn node(&self, id: NodeId) -> Option<&NodeSnapshot> {
        self.nodes.get(&id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubgraphSnapshot {
    pub id: SubgraphId,
    pub label: Option<String>,
    pub parent: Option<SubgraphId>,
    pub children: Vec<SubgraphId>,
    pub nodes: Vec<NodeId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeSnapshot {
    pub id: NodeId,
    pub label: Option<String>,
    pub subgraph_id: Option<SubgraphId>,
    pub kind: NodeKind,
    pub state: NodeState,
    pub value_type: Option<String>,
    pub debug_name: Option<String>,
    pub cached_value: Option<ValueSummary>,
    pub dependencies: Vec<NodeId>,
    pub dependents: Vec<NodeId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValueSummary {
    pub value_type: String,
    pub rendered: String,
    pub bytes_hex: String,
    pub comparison: ValueComparison,
}

impl ValueSummary {
    pub fn from_storage(value: &ValueStorage) -> Self {
        Self {
            value_type: value.value_type().name().to_string(),
            rendered: render_value(value),
            bytes_hex: bytes_hex(value.bytes()),
            comparison: value.comparison(),
        }
    }
}

fn render_value(value: &ValueStorage) -> String {
    match value.value_type().name() {
        "bool" => value
            .as_bool()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<invalid bool>".to_string()),
        "i64" => value
            .as_i64()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<invalid i64>".to_string()),
        "&'static str" | "String" => std::str::from_utf8(value.bytes())
            .map(|value| format!("{value:?}"))
            .unwrap_or_else(|_| "<invalid utf8>".to_string()),
        _ => format!("0x{}", bytes_hex(value.bytes())),
    }
}

fn bytes_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}
