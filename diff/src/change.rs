use attribute_graph::{Edge, EdgeState, NodeId, NodeState};

use crate::snapshot::{GraphSnapshot, NodeSnapshot, ValueSummary};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphDiff {
    pub before_label: String,
    pub after_label: String,
    pub changes: Vec<GraphChange>,
}

impl GraphDiff {
    pub fn between(before: &GraphSnapshot, after: &GraphSnapshot) -> Self {
        let mut changes = Vec::new();

        for (id, before_node) in &before.nodes {
            if !after.nodes.contains_key(id) {
                changes.push(GraphChange::NodeRemoved(before_node.clone()));
            }
        }

        for (id, after_node) in &after.nodes {
            match before.nodes.get(id) {
                Some(before_node) => {
                    changes.extend(diff_node(before_node, after_node));
                }
                None => changes.push(GraphChange::NodeAdded(after_node.clone())),
            }
        }

        for (edge, before_state) in &before.edges {
            match after.edges.get(edge) {
                Some(after_state) if before_state != after_state => {
                    changes.push(GraphChange::EdgeStateChanged {
                        edge: *edge,
                        before: *before_state,
                        after: *after_state,
                    });
                }
                Some(_) => {}
                None => changes.push(GraphChange::EdgeRemoved {
                    edge: *edge,
                    state: *before_state,
                }),
            }
        }

        for (edge, after_state) in &after.edges {
            if !before.edges.contains_key(edge) {
                changes.push(GraphChange::EdgeAdded {
                    edge: *edge,
                    state: *after_state,
                });
            }
        }

        Self {
            before_label: before.label.clone(),
            after_label: after.label.clone(),
            changes,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphChange {
    NodeAdded(NodeSnapshot),
    NodeRemoved(NodeSnapshot),
    NodeStateChanged {
        id: NodeId,
        before: NodeState,
        after: NodeState,
    },
    NodeValueChanged {
        id: NodeId,
        before: Option<ValueSummary>,
        after: Option<ValueSummary>,
    },
    EdgeAdded {
        edge: Edge,
        state: EdgeState,
    },
    EdgeRemoved {
        edge: Edge,
        state: EdgeState,
    },
    EdgeStateChanged {
        edge: Edge,
        before: EdgeState,
        after: EdgeState,
    },
}

fn diff_node(before: &NodeSnapshot, after: &NodeSnapshot) -> Vec<GraphChange> {
    let mut changes = Vec::new();

    if before.state != after.state {
        changes.push(GraphChange::NodeStateChanged {
            id: after.id,
            before: before.state,
            after: after.state,
        });
    }

    if before.cached_value != after.cached_value {
        changes.push(GraphChange::NodeValueChanged {
            id: after.id,
            before: before.cached_value.clone(),
            after: after.cached_value.clone(),
        });
    }

    changes
}
