use std::error::Error;
use std::fmt;

use crate::identity::{GraphId, NodeId};
use crate::value::TypeDescriptor;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphError {
    GraphMismatch {
        expected: GraphId,
        actual: GraphId,
    },
    MissingNode(NodeId),
    MissingValue(NodeId),
    MissingOutput(NodeId),
    NotSource(NodeId),
    NotDerived(NodeId),
    SelfDependency(NodeId),
    CycleDetected,
    RuleValueTypeMismatch {
        expected: TypeDescriptor,
        actual: TypeDescriptor,
    },
    ValueTypeMismatch {
        node: NodeId,
        expected: TypeDescriptor,
        actual: TypeDescriptor,
    },
    ValueDecodeFailed {
        node: NodeId,
        value_type: TypeDescriptor,
    },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GraphMismatch { expected, actual } => write!(
                f,
                "node belongs to graph {actual}, but this operation uses graph {expected}"
            ),
            Self::MissingNode(id) => write!(f, "missing node {id}"),
            Self::MissingValue(id) => write!(f, "node {id} has no cached value"),
            Self::MissingOutput(id) => {
                write!(f, "rule for node {id} did not set an output value")
            }
            Self::NotSource(id) => write!(f, "node {id} is not a source node"),
            Self::NotDerived(id) => write!(f, "node {id} is not a derived node"),
            Self::SelfDependency(id) => write!(f, "node {id} cannot depend on itself"),
            Self::CycleDetected => write!(f, "dependency cycle detected"),
            Self::RuleValueTypeMismatch { expected, actual } => write!(
                f,
                "rule expected to produce value type {}, got {}",
                expected.name(),
                actual.name()
            ),
            Self::ValueTypeMismatch {
                node,
                expected,
                actual,
            } => write!(
                f,
                "node {} expected value type {}, got {}",
                node,
                expected.name(),
                actual.name()
            ),
            Self::ValueDecodeFailed { node, value_type } => write!(
                f,
                "node {} has invalid cached bytes for value type {}",
                node,
                value_type.name()
            ),
        }
    }
}

impl Error for GraphError {}
