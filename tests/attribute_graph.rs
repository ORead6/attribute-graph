use std::any::type_name;

use attribute_graph::{
    Attribute, AttributeGraph, DependencyChangeSet, Edge, EdgeState, EvaluationContext, GraphError,
    NodeId, NodeKind, NodeState, RuleDescriptor, RuleHandle, TypeDescriptor, UpdateFn,
    UpdateOutcome, ValueComparison, ValueStorage,
};

const I64: TypeDescriptor = TypeDescriptor::new("i64");
const BOOL: TypeDescriptor = TypeDescriptor::new("bool");
const STATIC_STR: TypeDescriptor = TypeDescriptor::new("&'static str");

#[derive(Debug)]
struct ConstantRule {
    value: ValueStorage,
}

#[derive(Debug)]
struct SumRule {
    price: NodeId,
    quantity: NodeId,
}

#[derive(Debug)]
struct TypedSumRule {
    price: Attribute<i64>,
    quantity: Attribute<i64>,
}

#[derive(Debug)]
struct CappedRule {
    price: NodeId,
    cap: i64,
    comparison: ValueComparison,
}

#[derive(Debug)]
struct LabelRule {
    input: NodeId,
}

#[derive(Debug)]
struct ConditionalPriceRule {
    use_sale_price: NodeId,
    sale_price: NodeId,
    regular_price: NodeId,
}

#[derive(Debug)]
struct SelfReadingRule;

fn boxed_rule<T: 'static>(
    body: T,
    update: UpdateFn,
    value_type: TypeDescriptor,
    debug_name: &'static str,
) -> RuleDescriptor {
    let body = Box::new(body);
    let handle = RuleHandle::from_raw(Box::into_raw(body) as usize);

    RuleDescriptor::new(
        handle,
        update,
        TypeDescriptor::new(type_name::<T>()),
        value_type,
        debug_name,
    )
    .with_destroy(drop_boxed_rule::<T>)
}

fn drop_boxed_rule<T>(handle: RuleHandle) {
    // The graph never does this cast. This is the rule provider side proving the
    // opaque-body model: whoever supplied the body also supplies the function
    // that understands how to destroy it.
    unsafe {
        drop(Box::from_raw(handle.raw() as *mut T));
    }
}

fn rule_body<T>(handle: RuleHandle) -> &'static T {
    // Test callbacks stand in for a Swift/Rust bridge trampoline. The callback
    // receives the opaque handle and casts it back to the concrete rule body it
    // knows how to execute.
    unsafe { &*(handle.raw() as *const T) }
}

fn update_constant(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<ConstantRule>(handle);
    context.set_output(rule.value.clone());
    Ok(())
}

fn update_sum(handle: RuleHandle, context: &mut EvaluationContext<'_>) -> Result<(), GraphError> {
    let rule = rule_body::<SumRule>(handle);

    let price = context
        .read(rule.price)?
        .as_i64()
        .expect("price should be an i64");
    let quantity = context
        .read(rule.quantity)?
        .as_i64()
        .expect("quantity should be an i64");

    context.set_output(ValueStorage::from_i64(price * quantity));
    Ok(())
}

fn update_typed_sum(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<TypedSumRule>(handle);

    let price = context.read_attribute(rule.price)?;
    let quantity = context.read_attribute(rule.quantity)?;

    context.set_output_value(price * quantity);
    Ok(())
}

fn update_capped(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<CappedRule>(handle);
    let price = context
        .read(rule.price)?
        .as_i64()
        .expect("price should be an i64");

    context
        .set_output(ValueStorage::from_i64(price.min(rule.cap)).with_comparison(rule.comparison));
    Ok(())
}

fn update_label(handle: RuleHandle, context: &mut EvaluationContext<'_>) -> Result<(), GraphError> {
    let rule = rule_body::<LabelRule>(handle);
    let input = context
        .read(rule.input)?
        .as_i64()
        .expect("input should be an i64");

    let label = if input == 10 { "10" } else { "other" };
    context.set_output(ValueStorage::from_static_str(label));
    Ok(())
}

fn update_conditional_price(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<ConditionalPriceRule>(handle);

    let use_sale_price = context
        .read(rule.use_sale_price)?
        .as_bool()
        .expect("switch should be a bool");

    let selected_price = if use_sale_price {
        context
            .read(rule.sale_price)?
            .as_i64()
            .expect("sale price should be an i64")
    } else {
        context
            .read(rule.regular_price)?
            .as_i64()
            .expect("regular price should be an i64")
    };

    context.set_output(ValueStorage::from_i64(selected_price));
    Ok(())
}

fn update_self_reading(
    _handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    context.read(context.evaluating())?;
    Ok(())
}

#[test]
fn source_and_derived_nodes_store_different_runtime_data() {
    // What this checks:
    // - Source nodes store an externally supplied cached value and no rule.
    // - Derived nodes store no initial value, but they do store an opaque rule
    //   descriptor: body handle, update function, body type, output type, name.
    //
    // Why this matters:
    // The graph runtime can own dependency state without knowing what concrete
    // rule type will calculate a derived value later.
    let mut graph = AttributeGraph::new();

    let price = graph.add_source(ValueStorage::from_i64(10));
    let total = graph.add_derived(boxed_rule(
        ConstantRule {
            value: ValueStorage::from_i64(0),
        },
        update_constant,
        I64,
        "constant total",
    ));

    let price_node = graph.node(price).expect("source node should exist");
    assert_eq!(price_node.kind(), NodeKind::Source);
    assert_eq!(price_node.state(), NodeState::Clean);
    assert_eq!(graph.debug_cached_value(price).unwrap().as_i64(), Some(10));
    assert!(price_node.rule().is_none());

    let total_node = graph.node(total).expect("derived node should exist");
    assert_eq!(total_node.kind(), NodeKind::Derived);
    assert_eq!(total_node.state(), NodeState::Dirty);
    assert!(graph.debug_cached_value(total).is_none());
    assert_eq!(total_node.rule().unwrap().value_type(), I64);
    assert_eq!(total_node.rule().unwrap().debug_name(), "constant total");
}

#[test]
fn typed_static_and_dynamic_attributes_read_and_write_without_raw_storage() {
    // What this checks:
    // - Callers can create source-backed `StaticAttribute<T>` values.
    // - Rules can read typed attributes through `EvaluationContext`.
    // - Callers can read a `DynamicAttribute<T>` without touching `NodeId` or
    //   `ValueStorage` on the value path.
    let mut graph = AttributeGraph::new();

    let price = graph.add_static_attribute(10_i64);
    let quantity = graph.add_static_attribute(2_i64);
    let total = graph
        .add_dynamic_attribute::<i64>(boxed_rule(
            TypedSumRule {
                price: price.attribute(),
                quantity: quantity.attribute(),
            },
            update_typed_sum,
            I64,
            "typed price * quantity",
        ))
        .unwrap();

    assert_eq!(graph.read(total).unwrap(), 20);
    assert_eq!(
        graph.dependencies_of(total.id()),
        Ok(vec![price.id(), quantity.id()])
    );

    graph.set_static(price, 11).unwrap();

    assert_eq!(graph.node(total.id()).unwrap().state(), NodeState::Dirty);
    assert_eq!(graph.read(total).unwrap(), 22);
    assert_eq!(graph.node(total.id()).unwrap().state(), NodeState::Clean);
}

#[test]
fn graph_scoped_ids_reject_handles_from_another_graph() {
    let mut first = AttributeGraph::new();
    let first_value = first.add_static_attribute(10_i64);

    let mut second = AttributeGraph::new();
    let second_value = second.add_static_attribute(99_i64);

    assert_ne!(first.id(), second.id());
    assert_eq!(first_value.id().raw(), 0);
    assert_eq!(second_value.id().raw(), 0);
    assert_eq!(first_value.id().graph_id(), first.id());
    assert_eq!(second_value.id().graph_id(), second.id());
    assert_ne!(first_value.id(), second_value.id());
    assert!(!second.contains_node(first_value.id()));

    let expected_error = GraphError::GraphMismatch {
        expected: second.id(),
        actual: first.id(),
    };
    assert_eq!(second.read(first_value), Err(expected_error.clone()));
    assert_eq!(second.set_static(first_value, 11), Err(expected_error));
    assert_eq!(second.read(second_value), Ok(99));
}

#[test]
fn typed_dynamic_attributes_reject_rules_with_the_wrong_output_type() {
    let mut graph = AttributeGraph::new();

    let result = graph.add_dynamic_attribute::<i64>(boxed_rule(
        ConstantRule {
            value: ValueStorage::from_bool(true),
        },
        update_constant,
        BOOL,
        "bool rule",
    ));

    assert_eq!(
        result,
        Err(GraphError::RuleValueTypeMismatch {
            expected: I64,
            actual: BOOL,
        })
    );
}

#[test]
fn updating_a_derived_node_runs_its_external_rule_and_records_reads() {
    // What this checks:
    // - The graph does not contain "sum" logic.
    // - The derived node stores an opaque `SumRule` body and `update_sum`
    //   callback supplied by the rule provider.
    // - When the callback reads price and quantity through EvaluationContext,
    //   those reads become active dependency edges.
    //
    // Expected graph shape after evaluation:
    // - price -> total
    // - quantity -> total
    // - both edges are settled because total consumed the current values.
    let mut graph = AttributeGraph::new();

    let price = graph.add_source(ValueStorage::from_i64(10));
    let quantity = graph.add_source(ValueStorage::from_i64(2));
    let total = graph.add_derived(boxed_rule(
        SumRule { price, quantity },
        update_sum,
        I64,
        "price * quantity",
    ));

    let outcome = graph
        .update_node(total)
        .expect("external sum rule should run");

    assert_eq!(
        outcome,
        UpdateOutcome {
            dependency_changes: DependencyChangeSet {
                added: vec![price, quantity],
                removed: vec![],
                retained: vec![],
            },
            value_changed: true,
            dirtied_dependents: vec![],
        }
    );
    assert_eq!(graph.debug_cached_value(total).unwrap().as_i64(), Some(20));
    assert_eq!(graph.node(total).unwrap().state(), NodeState::Clean);
    assert_eq!(graph.dependencies_of(total), Ok(vec![price, quantity]));
    assert_eq!(graph.dependents_of(price), Ok(vec![total]));
    assert_eq!(graph.dependents_of(quantity), Ok(vec![total]));
    assert_eq!(
        graph.edges(),
        vec![Edge::new(price, total), Edge::new(quantity, total)]
    );
    assert_eq!(graph.edge_state(price, total), Ok(EdgeState::Settled));
    assert_eq!(graph.edge_state(quantity, total), Ok(EdgeState::Settled));
}

#[test]
fn source_changes_mark_direct_dependents_dirty_and_edges_pending() {
    // What this checks:
    // - Changing a dependency does not immediately recompute dependents.
    // - The direct dependent is marked dirty.
    // - The changed dependency edge becomes pending until the dependent's rule
    //   observes the new value.
    let mut graph = AttributeGraph::new();

    let price = graph.add_source(ValueStorage::from_i64(10));
    let quantity = graph.add_source(ValueStorage::from_i64(2));
    let total = graph.add_derived(boxed_rule(
        SumRule { price, quantity },
        update_sum,
        I64,
        "price * quantity",
    ));
    graph.update_node(total).unwrap();

    let dirtied = graph
        .set_source_value(price, ValueStorage::from_i64(11))
        .expect("writing a source should dirty direct dependents");

    assert_eq!(dirtied, vec![total]);
    assert_eq!(graph.node(total).unwrap().state(), NodeState::Dirty);
    assert_eq!(graph.pending_edges(), vec![Edge::new(price, total)]);
    assert_eq!(graph.edge_state(price, total), Ok(EdgeState::Pending));
    assert_eq!(graph.edge_state(quantity, total), Ok(EdgeState::Settled));
}

#[test]
fn reevaluating_a_dirty_node_clears_inbound_pending_edges() {
    // What this checks:
    // - A dirty derived node can consume a pending dependency change by running
    //   its stored external update callback.
    // - Once the callback has read its dependencies and set output, inbound
    //   pending edges are cleared.
    // - Because the value changed from 20 to 22, update_node reports a change.
    let mut graph = AttributeGraph::new();

    let price = graph.add_source(ValueStorage::from_i64(10));
    let quantity = graph.add_source(ValueStorage::from_i64(2));
    let total = graph.add_derived(boxed_rule(
        SumRule { price, quantity },
        update_sum,
        I64,
        "price * quantity",
    ));
    graph.update_node(total).unwrap();
    graph
        .set_source_value(price, ValueStorage::from_i64(11))
        .unwrap();

    let outcome = graph.update_node(total).unwrap();

    assert_eq!(
        outcome,
        UpdateOutcome {
            dependency_changes: DependencyChangeSet {
                added: vec![],
                removed: vec![],
                retained: vec![price, quantity],
            },
            value_changed: true,
            dirtied_dependents: vec![],
        }
    );
    assert_eq!(graph.debug_cached_value(total).unwrap().as_i64(), Some(22));
    assert_eq!(graph.node(total).unwrap().state(), NodeState::Clean);
    assert_eq!(graph.pending_edges(), vec![]);
    assert_eq!(graph.edge_state(price, total), Ok(EdgeState::Settled));
}

#[test]
fn recomputing_a_derived_node_does_not_dirty_dependents_when_value_is_unchanged() {
    // What this checks:
    // - The graph can recompute a dirty derived node and clear the pending input
    //   edge.
    // - Recomputing now compares the old and new cached values automatically.
    // - If the value is equal, downstream dependents stay valid.
    //
    // Scenario:
    // - capped_price = min(price, 10)
    // - label reads capped_price
    // - price changes from 12 to 13
    // - capped_price recomputes to the same value, 10
    let mut graph = AttributeGraph::new();

    let price = graph.add_source(ValueStorage::from_i64(12));
    let capped_price = graph.add_derived(boxed_rule(
        CappedRule {
            price,
            cap: 10,
            comparison: ValueComparison::Bytewise,
        },
        update_capped,
        I64,
        "min(price, 10)",
    ));
    let label = graph.add_derived(boxed_rule(
        LabelRule {
            input: capped_price,
        },
        update_label,
        STATIC_STR,
        "label capped price",
    ));
    graph.update_node(label).unwrap();

    graph
        .set_source_value(price, ValueStorage::from_i64(13))
        .unwrap();
    assert_eq!(graph.node(label).unwrap().state(), NodeState::MaybeDirty);

    let outcome = graph.update_node(capped_price).unwrap();

    assert_eq!(
        outcome,
        UpdateOutcome {
            dependency_changes: DependencyChangeSet {
                added: vec![],
                removed: vec![],
                retained: vec![price],
            },
            value_changed: false,
            dirtied_dependents: vec![],
        }
    );
    assert_eq!(
        graph.debug_cached_value(capped_price).unwrap().as_i64(),
        Some(10)
    );
    assert_eq!(graph.node(capped_price).unwrap().state(), NodeState::Clean);
    assert_eq!(graph.node(label).unwrap().state(), NodeState::Clean);
    assert_eq!(graph.pending_edges(), vec![]);
    assert_eq!(
        graph.edge_state(capped_price, label),
        Ok(EdgeState::Settled)
    );
}

#[test]
fn always_changed_comparison_policy_forces_downstream_invalidation() {
    // What this checks:
    // - The comparison policy lives with the produced value.
    // - `AlwaysChanged` is useful for values where the host says every
    //   recomputation should invalidate dependents even if bytes match.
    // - This is the simple built-in version of the future host-supplied
    //   comparison callback described in the library comments.
    let mut graph = AttributeGraph::new();

    let price = graph.add_source(ValueStorage::from_i64(12));
    let capped_price = graph.add_derived(boxed_rule(
        CappedRule {
            price,
            cap: 10,
            comparison: ValueComparison::AlwaysChanged,
        },
        update_capped,
        I64,
        "always changed capped price",
    ));
    let label = graph.add_derived(boxed_rule(
        LabelRule {
            input: capped_price,
        },
        update_label,
        STATIC_STR,
        "label capped price",
    ));
    graph.update_node(label).unwrap();

    graph
        .set_source_value(price, ValueStorage::from_i64(13))
        .unwrap();
    let outcome = graph.update_node(capped_price).unwrap();

    assert!(outcome.value_changed);
    assert_eq!(outcome.dirtied_dependents, vec![label]);
    assert_eq!(graph.node(label).unwrap().state(), NodeState::Dirty);
    assert_eq!(graph.pending_edges(), vec![Edge::new(capped_price, label)]);
}

#[test]
fn reading_maybe_dirty_descendant_validates_dirty_ancestors_lazily() {
    // What this checks:
    // - A source change dirties the direct derived node, but only marks deeper
    //   descendants maybe-dirty.
    // - Reading the descendant validates the dirty ancestor first.
    // - The descendant only recomputes if that ancestor's cached value changed.
    let mut graph = AttributeGraph::new();

    let price = graph.add_source(ValueStorage::from_i64(10));
    let quantity = graph.add_source(ValueStorage::from_i64(2));
    let multiplier = graph.add_source(ValueStorage::from_i64(3));
    let total = graph.add_derived(boxed_rule(
        SumRule { price, quantity },
        update_sum,
        I64,
        "price * quantity",
    ));
    let scaled_total = graph.add_derived(boxed_rule(
        SumRule {
            price: total,
            quantity: multiplier,
        },
        update_sum,
        I64,
        "total * multiplier",
    ));
    graph.update_node(scaled_total).unwrap();

    graph
        .set_source_value(price, ValueStorage::from_i64(11))
        .unwrap();

    assert_eq!(graph.node(total).unwrap().state(), NodeState::Dirty);
    assert_eq!(
        graph.node(scaled_total).unwrap().state(),
        NodeState::MaybeDirty
    );
    assert_eq!(
        graph.pending_edges(),
        vec![Edge::new(price, total), Edge::new(total, scaled_total)]
    );

    let value = graph.read_value(scaled_total).unwrap();

    assert_eq!(value.as_i64(), Some(66));
    assert_eq!(graph.node(total).unwrap().state(), NodeState::Clean);
    assert_eq!(graph.node(scaled_total).unwrap().state(), NodeState::Clean);
    assert_eq!(graph.pending_edges(), vec![]);
}

#[test]
fn changed_derived_nodes_automatically_dirty_their_own_dependents() {
    // What this checks:
    // - A derived node can itself be a dependency of another derived node.
    // - After `total` recomputes, bytewise comparison detects whether its cached
    //   value changed.
    // - When it did change, the graph automatically dirties direct dependents.
    let mut graph = AttributeGraph::new();

    let price = graph.add_source(ValueStorage::from_i64(10));
    let quantity = graph.add_source(ValueStorage::from_i64(2));
    let total = graph.add_derived(boxed_rule(
        SumRule { price, quantity },
        update_sum,
        I64,
        "price * quantity",
    ));
    let label = graph.add_derived(boxed_rule(
        LabelRule { input: total },
        update_label,
        STATIC_STR,
        "label total",
    ));
    graph.update_node(label).unwrap();

    graph
        .set_source_value(price, ValueStorage::from_i64(11))
        .unwrap();
    let outcome = graph.update_node(total).unwrap();

    assert_eq!(
        outcome,
        UpdateOutcome {
            dependency_changes: DependencyChangeSet {
                added: vec![],
                removed: vec![],
                retained: vec![price, quantity],
            },
            value_changed: true,
            dirtied_dependents: vec![label],
        }
    );
    assert_eq!(graph.node(total).unwrap().state(), NodeState::Clean);
    assert_eq!(graph.node(label).unwrap().state(), NodeState::Dirty);
    assert_eq!(graph.pending_edges(), vec![Edge::new(total, label)]);
    assert_eq!(graph.edge_state(price, total), Ok(EdgeState::Settled));
    assert_eq!(graph.edge_state(total, label), Ok(EdgeState::Pending));
}

#[test]
fn conditional_rules_replace_their_active_dependency_set() {
    // What this checks:
    // - Conditional rules are why dependencies need to be discovered during
    //   evaluation rather than hardcoded once.
    // - The rule always reads use_sale_price.
    // - It reads sale_price only on the true branch.
    // - It reads regular_price only on the false branch.
    let mut graph = AttributeGraph::new();

    let use_sale_price = graph.add_source(ValueStorage::from_bool(true));
    let sale_price = graph.add_source(ValueStorage::from_i64(7));
    let regular_price = graph.add_source(ValueStorage::from_i64(10));
    let selected_price = graph.add_derived(boxed_rule(
        ConditionalPriceRule {
            use_sale_price,
            sale_price,
            regular_price,
        },
        update_conditional_price,
        I64,
        "selected price",
    ));

    graph.update_node(selected_price).unwrap();
    assert_eq!(
        graph.dependencies_of(selected_price),
        Ok(vec![use_sale_price, sale_price])
    );

    graph
        .set_source_value(use_sale_price, ValueStorage::from_bool(false))
        .unwrap();
    let outcome = graph.update_node(selected_price).unwrap();

    assert_eq!(
        outcome,
        UpdateOutcome {
            dependency_changes: DependencyChangeSet {
                added: vec![regular_price],
                removed: vec![sale_price],
                retained: vec![use_sale_price],
            },
            value_changed: true,
            dirtied_dependents: vec![],
        }
    );
    assert_eq!(
        graph.dependencies_of(selected_price),
        Ok(vec![use_sale_price, regular_price])
    );
    assert_eq!(
        graph.edge_state(sale_price, selected_price),
        Ok(EdgeState::Inactive)
    );
    assert_eq!(
        graph.edge_state(regular_price, selected_price),
        Ok(EdgeState::Settled)
    );

    graph
        .set_source_value(sale_price, ValueStorage::from_i64(6))
        .unwrap();
    assert_eq!(
        graph.node(selected_price).unwrap().state(),
        NodeState::Clean,
        "changing an inactive dependency should not dirty the derived node",
    );

    graph
        .set_source_value(regular_price, ValueStorage::from_i64(11))
        .unwrap();
    assert_eq!(
        graph.node(selected_price).unwrap().state(),
        NodeState::Dirty,
        "changing the active branch should dirty the derived node",
    );
}

#[test]
fn nested_reads_update_dirty_derived_dependencies_before_returning_values() {
    // What this checks:
    // - A rule can read another derived node.
    // - If that dependency is dirty, the graph updates it first.
    // - The current rule then reads a fresh cached value and records the edge.
    let mut graph = AttributeGraph::new();

    let price = graph.add_source(ValueStorage::from_i64(10));
    let quantity = graph.add_source(ValueStorage::from_i64(2));
    let total = graph.add_derived(boxed_rule(
        SumRule { price, quantity },
        update_sum,
        I64,
        "price * quantity",
    ));
    let label = graph.add_derived(boxed_rule(
        LabelRule { input: total },
        update_label,
        STATIC_STR,
        "label total",
    ));

    graph.update_node(label).unwrap();

    assert_eq!(graph.debug_cached_value(total).unwrap().as_i64(), Some(20));
    assert_eq!(
        graph.debug_cached_value(label).unwrap().as_static_str(),
        Some("other")
    );
    assert_eq!(graph.dependencies_of(total), Ok(vec![price, quantity]));
    assert_eq!(graph.dependencies_of(label), Ok(vec![total]));
}

#[test]
fn output_type_is_checked_against_the_rule_descriptor() {
    // What this checks:
    // - The rule descriptor declares the derived node's output type.
    // - The callback can come from outside the graph, so the graph validates the
    //   value it writes before caching it.
    let mut graph = AttributeGraph::new();

    let bad = graph.add_derived(boxed_rule(
        ConstantRule {
            value: ValueStorage::from_bool(true),
        },
        update_constant,
        I64,
        "bad bool as i64",
    ));

    let result = graph.update_node(bad);

    assert_eq!(
        result,
        Err(GraphError::ValueTypeMismatch {
            node: bad,
            expected: I64,
            actual: BOOL,
        })
    );
    assert!(graph.debug_cached_value(bad).is_none());
    assert_eq!(graph.node(bad).unwrap().state(), NodeState::Dirty);
}

#[test]
fn a_rule_cannot_read_its_own_output() {
    // What this checks:
    // - Public dependency discovery rejects a self-dependency during evaluation.
    // - The failed read leaves the node dirty and does not commit an edge.
    let mut graph = AttributeGraph::new();
    let derived = graph.add_derived(boxed_rule(
        SelfReadingRule,
        update_self_reading,
        I64,
        "self-reading rule",
    ));

    assert_eq!(
        graph.update_node(derived),
        Err(GraphError::SelfDependency(derived))
    );
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Dirty);
    assert_eq!(graph.dependencies_of(derived), Ok(vec![]));
    assert!(graph.edges().is_empty());
}

#[test]
fn removing_a_node_removes_active_and_pending_edges_touching_it() {
    // What this checks:
    // - Node lifetime still matters with rule-backed derived nodes.
    // - Removing a node cleans dependency edges where it is either dependency or
    //   dependent.
    // - Pending edge state is cleaned up too.
    let mut graph = AttributeGraph::new();

    let price = graph.add_source(ValueStorage::from_i64(10));
    let quantity = graph.add_source(ValueStorage::from_i64(2));
    let total = graph.add_derived(boxed_rule(
        SumRule { price, quantity },
        update_sum,
        I64,
        "price * quantity",
    ));
    let label = graph.add_derived(boxed_rule(
        LabelRule { input: total },
        update_label,
        STATIC_STR,
        "label total",
    ));
    graph.update_node(label).unwrap();
    graph
        .set_source_value(price, ValueStorage::from_i64(11))
        .unwrap();

    let removed = graph
        .remove_node(total)
        .expect("total node should be removable");

    assert_eq!(removed.id(), total);
    assert_eq!(graph.node_count(), 3);
    assert_eq!(graph.edge_count(), 0);
    assert_eq!(graph.pending_edges(), vec![]);
    assert_eq!(graph.dependents_of(price), Ok(vec![]));
    assert_eq!(graph.dependents_of(quantity), Ok(vec![]));
    assert_eq!(graph.dependencies_of(label), Ok(vec![]));
    assert_eq!(graph.node(label).unwrap().state(), NodeState::Dirty);
    assert_eq!(graph.read_value(label), Err(GraphError::MissingNode(total)));
}

#[test]
fn removing_a_dependency_never_returns_a_stale_cached_value() {
    let mut graph = AttributeGraph::new();

    let price = graph.add_source(ValueStorage::from_i64(10));
    let quantity = graph.add_source(ValueStorage::from_i64(2));
    let total = graph.add_derived(boxed_rule(
        SumRule { price, quantity },
        update_sum,
        I64,
        "price * quantity",
    ));
    let label = graph.add_derived(boxed_rule(
        LabelRule { input: total },
        update_label,
        STATIC_STR,
        "label total",
    ));

    graph.update_node(label).unwrap();
    assert_eq!(graph.read_value(total).unwrap().as_i64(), Some(20));

    graph.remove_node(price).unwrap();

    assert_eq!(graph.node(total).unwrap().state(), NodeState::Dirty);
    assert_eq!(graph.node(label).unwrap().state(), NodeState::MaybeDirty);
    assert_eq!(graph.read_value(total), Err(GraphError::MissingNode(price)));
    assert_eq!(graph.read_value(label), Err(GraphError::MissingNode(price)));
}
