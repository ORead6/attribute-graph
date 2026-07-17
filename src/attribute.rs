use std::marker::PhantomData;

use crate::error::GraphError;
use crate::identity::NodeId;
use crate::value::{AttributeValue, ValueStorage};

/// A typed handle to a graph node that stores or produces `T`.
///
/// This is the public identity layer callers should pass around instead of raw
/// `NodeId`s. Static and dynamic attributes both erase to this common handle so
/// rules can depend on either kind without caring how the value is produced.
#[derive(Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Attribute<T> {
    id: NodeId,
    _value: PhantomData<fn() -> T>,
}

impl<T> Clone for Attribute<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Attribute<T> {}

impl<T> Attribute<T> {
    pub(crate) const fn new(id: NodeId) -> Self {
        Self {
            id,
            _value: PhantomData,
        }
    }

    pub const fn id(self) -> NodeId {
        self.id
    }
}

/// A typed source attribute whose value is supplied from outside the graph.
#[derive(Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StaticAttribute<T> {
    attribute: Attribute<T>,
}

impl<T> Clone for StaticAttribute<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for StaticAttribute<T> {}

impl<T> StaticAttribute<T> {
    pub(crate) const fn new(id: NodeId) -> Self {
        Self {
            attribute: Attribute::new(id),
        }
    }

    pub const fn id(self) -> NodeId {
        self.attribute.id()
    }

    pub const fn attribute(self) -> Attribute<T> {
        self.attribute
    }
}

impl<T> From<StaticAttribute<T>> for Attribute<T> {
    fn from(attribute: StaticAttribute<T>) -> Self {
        attribute.attribute
    }
}

/// A typed derived attribute whose value is produced by a rule.
#[derive(Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DynamicAttribute<T> {
    attribute: Attribute<T>,
}

impl<T> Clone for DynamicAttribute<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for DynamicAttribute<T> {}

impl<T> DynamicAttribute<T> {
    pub(crate) const fn new(id: NodeId) -> Self {
        Self {
            attribute: Attribute::new(id),
        }
    }

    pub const fn id(self) -> NodeId {
        self.attribute.id()
    }

    pub const fn attribute(self) -> Attribute<T> {
        self.attribute
    }
}

impl<T> From<DynamicAttribute<T>> for Attribute<T> {
    fn from(attribute: DynamicAttribute<T>) -> Self {
        attribute.attribute
    }
}

pub(crate) fn decode_attribute_value<T: AttributeValue>(
    id: NodeId,
    value: &ValueStorage,
) -> Result<T, GraphError> {
    let expected = T::type_descriptor();
    let actual = value.value_type();

    if expected != actual {
        return Err(GraphError::ValueTypeMismatch {
            node: id,
            expected,
            actual,
        });
    }

    T::from_storage(value).ok_or(GraphError::ValueDecodeFailed {
        node: id,
        value_type: expected,
    })
}
