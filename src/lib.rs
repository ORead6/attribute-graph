//! A small, lazy attribute graph runtime.
//!
//! The crate root is intentionally a public facade. Implementation details are
//! grouped into private modules by responsibility, while the original
//! `attribute_graph::TypeName` API remains available through explicit re-exports.

mod attribute;
mod dependency;
mod error;
mod graph;
mod identity;
mod node;
mod rule;
mod value;

pub use attribute::{Attribute, DynamicAttribute, StaticAttribute};
pub use dependency::{DependencyChangeSet, Edge, EdgeState, UpdateOutcome};
pub use error::GraphError;
pub use graph::{AttributeGraph, EvaluationContext};
pub use identity::{GraphId, NodeId};
pub use node::{Node, NodeKind, NodeState};
pub use rule::{DestroyFn, RuleDescriptor, RuleHandle, UpdateFn};
pub use value::{AttributeValue, TypeDescriptor, ValueComparison, ValueStorage};
