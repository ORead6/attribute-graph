//! Compile-time coverage for the crate-root facade.

use attribute_graph::{
    Attribute, AttributeGraph, AttributeValue, DependencyChangeSet, DestroyFn, DynamicAttribute,
    Edge, EdgeState, EvaluationContext, GraphError, GraphId, Node, NodeId, NodeKind, NodeState,
    RuleDescriptor, RuleHandle, StaticAttribute, TypeDescriptor, UpdateFn, UpdateOutcome,
    ValueComparison, ValueStorage,
};

fn assert_public_type<T>() {}

fn assert_attribute_value<T: AttributeValue>() {}

#[test]
fn original_crate_root_exports_remain_available() {
    assert_public_type::<Attribute<i64>>();
    assert_public_type::<AttributeGraph>();
    assert_public_type::<DependencyChangeSet>();
    assert_public_type::<DynamicAttribute<i64>>();
    assert_public_type::<Edge>();
    assert_public_type::<EdgeState>();
    assert_public_type::<EvaluationContext<'static>>();
    assert_public_type::<GraphError>();
    assert_public_type::<GraphId>();
    assert_public_type::<Node>();
    assert_public_type::<NodeId>();
    assert_public_type::<NodeKind>();
    assert_public_type::<NodeState>();
    assert_public_type::<RuleDescriptor>();
    assert_public_type::<RuleHandle>();
    assert_public_type::<StaticAttribute<i64>>();
    assert_public_type::<TypeDescriptor>();
    assert_public_type::<UpdateOutcome>();
    assert_public_type::<ValueComparison>();
    assert_public_type::<ValueStorage>();

    assert_attribute_value::<bool>();
    assert_attribute_value::<i64>();
    assert_attribute_value::<String>();

    let _: Option<DestroyFn> = None;
    let _: Option<UpdateFn> = None;
}
