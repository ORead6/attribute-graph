use std::collections::BTreeMap;

use attribute_graph::{Attribute, AttributeGraph, GraphError, NodeId};

use crate::{GraphDiff, GraphSnapshot};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiffSession {
    snapshots: Vec<GraphSnapshot>,
    diffs: Vec<GraphDiff>,
    node_labels: BTreeMap<NodeId, String>,
}

impl DiffSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn label_node(&mut self, id: NodeId, label: impl Into<String>) {
        self.node_labels.insert(id, label.into());
    }

    pub fn label_attribute<T>(&mut self, attribute: Attribute<T>, label: impl Into<String>) {
        self.label_node(attribute.id(), label);
    }

    pub fn capture(
        &mut self,
        label: impl Into<String>,
        graph: &AttributeGraph,
    ) -> Result<&GraphSnapshot, GraphError> {
        let snapshot = GraphSnapshot::capture_with_labels(label, graph, &self.node_labels)?;

        if let Some(before) = self.snapshots.last() {
            self.diffs.push(GraphDiff::between(before, &snapshot));
        }

        self.snapshots.push(snapshot);
        Ok(self.snapshots.last().expect("snapshot was just pushed"))
    }

    pub fn snapshots(&self) -> &[GraphSnapshot] {
        &self.snapshots
    }

    pub fn diffs(&self) -> &[GraphDiff] {
        &self.diffs
    }

    pub fn latest_snapshot(&self) -> Option<&GraphSnapshot> {
        self.snapshots.last()
    }

    pub fn latest_diff(&self) -> Option<&GraphDiff> {
        self.diffs.last()
    }
}
