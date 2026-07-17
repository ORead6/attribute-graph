use std::collections::HashSet;

use crate::identity::NodeId;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Edge {
    // `dependent` depends on `dependency`.
    // If `dependency` changes, `dependent` may need to be recomputed.
    pub dependency: NodeId,
    pub dependent: NodeId,
}

impl Edge {
    pub const fn new(dependency: NodeId, dependent: NodeId) -> Self {
        Self {
            dependency,
            dependent,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EdgeState {
    Inactive,
    Settled,
    Pending,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DependencyChangeSet {
    pub added: Vec<NodeId>,
    pub removed: Vec<NodeId>,
    pub retained: Vec<NodeId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UpdateOutcome {
    pub dependency_changes: DependencyChangeSet,
    pub value_changed: bool,
    pub dirtied_dependents: Vec<NodeId>,
}

pub(crate) fn sorted_ids(ids: Option<&HashSet<NodeId>>) -> Vec<NodeId> {
    let mut ids = ids.into_iter().flatten().copied().collect::<Vec<NodeId>>();
    ids.sort();
    ids
}
