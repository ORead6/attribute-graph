use std::collections::BTreeMap;

use attribute_graph::{
    AttributeGraph, Edge, EdgeState, GraphError, NodeId, NodeKind, NodeState, ValueComparison,
    ValueStorage,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphSnapshot {
    pub label: String,
    pub nodes: BTreeMap<NodeId, NodeSnapshot>,
    pub edges: BTreeMap<Edge, EdgeState>,
    pub pending_edges: Vec<Edge>,
}

impl GraphSnapshot {
    pub fn capture(label: impl Into<String>, graph: &AttributeGraph) -> Result<Self, GraphError> {
        let mut nodes = BTreeMap::new();

        for id in graph.topological_order()? {
            let node = graph.node(id).ok_or(GraphError::MissingNode(id))?;
            let rule = node.rule();

            nodes.insert(
                id,
                NodeSnapshot {
                    id,
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
pub struct NodeSnapshot {
    pub id: NodeId,
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
