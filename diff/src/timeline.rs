use attribute_graph::{AttributeGraph, GraphError};

use crate::{GraphDiff, GraphSnapshot};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiffSession {
    snapshots: Vec<GraphSnapshot>,
    diffs: Vec<GraphDiff>,
}

impl DiffSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn capture(
        &mut self,
        label: impl Into<String>,
        graph: &AttributeGraph,
    ) -> Result<&GraphSnapshot, GraphError> {
        let snapshot = GraphSnapshot::capture(label, graph)?;

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
