use std::collections::HashSet;

use crate::identity::{NodeId, SubgraphId};

/// An inspectable ownership scope within an [`crate::AttributeGraph`].
///
/// A subgraph groups nodes for lifecycle management. Its children and nodes are
/// exposed as sorted snapshots so inspection remains deterministic while the
/// graph keeps direct ownership of the mutable membership sets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Subgraph {
    pub(crate) id: SubgraphId,
    pub(crate) parent: Option<SubgraphId>,
    pub(crate) children: HashSet<SubgraphId>,
    pub(crate) nodes: HashSet<NodeId>,
}

impl Subgraph {
    /// Return this subgraph's stable identity.
    pub const fn id(&self) -> SubgraphId {
        self.id
    }

    /// Return the owning parent, or `None` for a root subgraph.
    pub const fn parent(&self) -> Option<SubgraphId> {
        self.parent
    }

    /// Return this subgraph's direct children in stable identity order.
    pub fn children(&self) -> Vec<SubgraphId> {
        sorted(self.children.iter().copied())
    }

    /// Return the nodes directly owned by this subgraph in stable identity order.
    pub fn nodes(&self) -> Vec<NodeId> {
        sorted(self.nodes.iter().copied())
    }
}

/// Summary of one completed subgraph removal.
///
/// Each vector is sorted and deduplicated. `subgraphs` includes the requested
/// subgraph and every descendant removed with it. `nodes` contains their former
/// node identities, and `dirtied_dependents` contains surviving nodes that were
/// marked or kept dirty because they directly depended on removed nodes. It is
/// not a state-transition notification and does not include transitive
/// maybe-dirty descendants.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SubgraphRemoval {
    pub subgraphs: Vec<SubgraphId>,
    pub nodes: Vec<NodeId>,
    pub dirtied_dependents: Vec<NodeId>,
}

fn sorted<T: Copy + Ord>(values: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort();
    values
}
