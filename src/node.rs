use std::collections::HashSet;

use crate::dependency::sorted_ids;
use crate::identity::{NodeId, SubgraphId};
use crate::rule::RuleDescriptor;
use crate::value::{TypeDescriptor, ValueStorage};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    /// Externally supplied state.
    Source,
    /// Calculated from a rule and the attributes that rule reads.
    Derived,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeState {
    /// The cached value is current after the node's dependencies have been validated.
    Clean,
    /// The cached value may be stale and should be recomputed before use.
    Dirty,
    /// A transitive dependency may be stale, so reads must validate inputs first.
    ///
    /// This is the key lazy AttributeGraph state: when `A -> B -> C` and `A`
    /// changes, `B` is definitely dirty but `C` is only maybe dirty. Reading `C`
    /// first validates `B`; if `B` recomputes to the same value, `C` can stay
    /// clean without running its own rule.
    MaybeDirty,
}

/// Internal node storage.
///
/// Source nodes have a value and no rule. Derived nodes have an optional cached
/// value plus a `RuleDescriptor`. Active dependencies are stored on the
/// dependent node so they can be replaced after each evaluation, which is what
/// eventually makes conditional dependencies work.
#[derive(Debug)]
pub struct Node {
    pub(crate) id: NodeId,
    pub(crate) subgraph: Option<SubgraphId>,
    pub(crate) kind: NodeKind,
    pub(crate) state: NodeState,
    pub(crate) value: Option<ValueStorage>,
    pub(crate) active_dependencies: HashSet<NodeId>,
    pub(crate) rule: Option<RuleDescriptor>,
}

impl Node {
    pub const fn id(&self) -> NodeId {
        self.id
    }

    /// Return the optional subgraph that owns this node.
    ///
    /// Nodes created outside a subgraph return `None`.
    pub const fn subgraph_id(&self) -> Option<SubgraphId> {
        self.subgraph
    }

    pub const fn kind(&self) -> NodeKind {
        self.kind
    }

    pub const fn state(&self) -> NodeState {
        self.state
    }

    pub const fn is_dirty(&self) -> bool {
        matches!(self.state, NodeState::Dirty)
    }

    #[doc(hidden)]
    pub fn debug_cached_value(&self) -> Option<&ValueStorage> {
        self.value.as_ref()
    }

    pub fn value_type(&self) -> Option<TypeDescriptor> {
        match self.kind {
            NodeKind::Source => self.value.as_ref().map(ValueStorage::value_type),
            NodeKind::Derived => self.rule.as_ref().map(RuleDescriptor::value_type),
        }
    }

    pub fn rule(&self) -> Option<&RuleDescriptor> {
        self.rule.as_ref()
    }

    pub fn active_dependencies(&self) -> Vec<NodeId> {
        sorted_ids(Some(&self.active_dependencies))
    }
}
