use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_GRAPH_ID: AtomicU64 = AtomicU64::new(1);

/// Stable identity for one [`crate::AttributeGraph`] instance.
///
/// Graph ids make it possible to reject a node handle from another graph even
/// when both graphs have assigned the same graph-local node number.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GraphId(u64);

impl GraphId {
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for GraphId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "g{}", self.raw())
    }
}

/// Stable handle for a node inside the graph.
///
/// The graph owns the actual node storage. `NodeId` is intentionally tiny and
/// copyable so external layers can hold onto handles without borrowing the
/// graph. A Swift bridge, for example, could store this beside an Attribute
/// value and pass it back when it wants to read or update that attribute.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId {
    graph: GraphId,
    local: u64,
}

impl NodeId {
    pub(crate) const fn new(graph: GraphId, local: u64) -> Self {
        Self { graph, local }
    }

    /// Return the graph that owns this node.
    pub const fn graph_id(self) -> GraphId {
        self.graph
    }

    /// Return the graph-local node number.
    ///
    /// This number is useful for compact labels, but it is not globally unique.
    /// Use the complete `NodeId` when storing or comparing node identities.
    pub const fn raw(self) -> u64 {
        self.local
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:n{}", self.graph_id(), self.raw())
    }
}

/// Stable handle for an ownership scope inside one attribute graph.
///
/// Like [`NodeId`], a subgraph id combines the owning graph's identity with a
/// graph-local number. The graph allocates those local numbers monotonically
/// and does not reuse them, so a handle cannot silently begin referring to a
/// later subgraph after its original scope is removed.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SubgraphId {
    graph: GraphId,
    local: u64,
}

impl SubgraphId {
    pub(crate) const fn new(graph: GraphId, local: u64) -> Self {
        Self { graph, local }
    }

    /// Return the graph that owns this subgraph.
    pub const fn graph_id(self) -> GraphId {
        self.graph
    }

    /// Return the graph-local subgraph number.
    ///
    /// This number is useful for compact labels, but it is not globally unique.
    /// Use the complete `SubgraphId` when storing or comparing identities.
    pub const fn raw(self) -> u64 {
        self.local
    }
}

impl fmt::Display for SubgraphId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:s{}", self.graph_id(), self.raw())
    }
}

pub(crate) fn next_graph_id() -> GraphId {
    NEXT_GRAPH_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map(GraphId)
        .expect("attribute graph exhausted its graph id space")
}
