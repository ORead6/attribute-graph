use std::any::type_name;
use std::cell::Cell;
use std::rc::Rc;

use attribute_graph::{
    AttributeGraph, Edge, EdgeState, EvaluationContext, GraphError, NodeId, NodeState,
    RuleDescriptor, RuleHandle, TypeDescriptor, UpdateFn, ValueComparison, ValueStorage,
};

const I64: TypeDescriptor = TypeDescriptor::new("i64");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NextAction {
    Succeed,
    ReturnError,
    OmitOutput,
}

struct ControlledRule {
    input: Rc<Cell<NodeId>>,
    next_action: Rc<Cell<NextAction>>,
    updates: Rc<Cell<usize>>,
}

struct ControlledRuleControl {
    input: Rc<Cell<NodeId>>,
    next_action: Rc<Cell<NextAction>>,
    updates: Rc<Cell<usize>>,
}

struct CappedRule {
    input: NodeId,
    cap: i64,
}

struct ProductRule {
    lhs: NodeId,
    rhs: NodeId,
    updates: Rc<Cell<usize>>,
}

struct DropTrackedRule {
    drops: Rc<Cell<usize>>,
    value: i64,
}

impl Drop for DropTrackedRule {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

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
    unsafe {
        drop(Box::from_raw(handle.raw() as *mut T));
    }
}

fn rule_body<T>(handle: RuleHandle) -> &'static T {
    unsafe { &*(handle.raw() as *const T) }
}

fn update_controlled(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<ControlledRule>(handle);
    rule.updates.set(rule.updates.get() + 1);

    let value = context.read(rule.input.get())?;

    match rule.next_action.replace(NextAction::Succeed) {
        NextAction::Succeed => context.set_output(value),
        // The graph propagates callback errors without interpreting them. This
        // sentinel stands in for an error reported by an external rule provider.
        // Setting output first proves that an error still aborts the whole attempt.
        NextAction::ReturnError => {
            context.set_output(value);
            return Err(GraphError::CycleDetected);
        }
        NextAction::OmitOutput => {}
    }

    Ok(())
}

fn update_capped(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<CappedRule>(handle);
    let input = context
        .read(rule.input)?
        .as_i64()
        .expect("capped input should be an i64");

    context.set_output(
        ValueStorage::from_i64(input.min(rule.cap)).with_comparison(ValueComparison::Bytewise),
    );
    Ok(())
}

fn update_product(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<ProductRule>(handle);
    rule.updates.set(rule.updates.get() + 1);

    let lhs = context
        .read(rule.lhs)?
        .as_i64()
        .expect("left input should be an i64");
    let rhs = context
        .read(rule.rhs)?
        .as_i64()
        .expect("right input should be an i64");

    context.set_output(ValueStorage::from_i64(lhs * rhs));
    Ok(())
}

fn update_drop_tracked(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<DropTrackedRule>(handle);
    context.set_output(ValueStorage::from_i64(rule.value));
    Ok(())
}

fn controlled_rule(input: NodeId) -> (ControlledRule, ControlledRuleControl) {
    let input = Rc::new(Cell::new(input));
    let next_action = Rc::new(Cell::new(NextAction::Succeed));
    let updates = Rc::new(Cell::new(0));

    let control = ControlledRuleControl {
        input,
        next_action,
        updates,
    };

    (
        ControlledRule {
            input: Rc::clone(&control.input),
            next_action: Rc::clone(&control.next_action),
            updates: Rc::clone(&control.updates),
        },
        control,
    )
}

#[test]
fn callback_error_does_not_commit_partial_evaluation_and_can_retry() {
    let mut graph = AttributeGraph::new();
    let original_input = graph.add_source(ValueStorage::from_i64(10));
    let replacement_input = graph.add_source(ValueStorage::from_i64(20));
    let (rule, control) = controlled_rule(original_input);
    let derived = graph.add_derived(boxed_rule(
        rule,
        update_controlled,
        I64,
        "controlled pass-through",
    ));

    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(10));
    graph
        .set_source_value(original_input, ValueStorage::from_i64(11))
        .unwrap();
    control.input.set(replacement_input);
    control.next_action.set(NextAction::ReturnError);

    assert_eq!(graph.update_node(derived), Err(GraphError::CycleDetected));
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Dirty);
    assert_eq!(
        graph.debug_cached_value(derived).unwrap().as_i64(),
        Some(10),
        "the old cache stays internal while the node remains dirty",
    );
    assert_eq!(graph.dependencies_of(derived), Ok(vec![original_input]));
    assert_eq!(graph.dependents_of(replacement_input), Ok(vec![]));
    assert_eq!(
        graph.pending_edges(),
        vec![Edge::new(original_input, derived)]
    );
    assert_eq!(control.updates.get(), 2);

    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(20));
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Clean);
    assert_eq!(graph.dependencies_of(derived), Ok(vec![replacement_input]));
    assert!(graph.pending_edges().is_empty());
    assert_eq!(control.updates.get(), 3);
}

#[test]
fn missing_output_preserves_previous_state_and_can_retry() {
    let mut graph = AttributeGraph::new();
    let original_input = graph.add_source(ValueStorage::from_i64(10));
    let replacement_input = graph.add_source(ValueStorage::from_i64(20));
    let (rule, control) = controlled_rule(original_input);
    let derived = graph.add_derived(boxed_rule(
        rule,
        update_controlled,
        I64,
        "occasionally omits output",
    ));

    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(10));
    graph
        .set_source_value(original_input, ValueStorage::from_i64(11))
        .unwrap();
    control.input.set(replacement_input);
    control.next_action.set(NextAction::OmitOutput);

    assert_eq!(
        graph.update_node(derived),
        Err(GraphError::MissingOutput(derived))
    );
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Dirty);
    assert_eq!(
        graph.debug_cached_value(derived).unwrap().as_i64(),
        Some(10)
    );
    assert_eq!(graph.dependencies_of(derived), Ok(vec![original_input]));
    assert_eq!(graph.dependents_of(replacement_input), Ok(vec![]));
    assert_eq!(
        graph.edge_state(original_input, derived),
        Ok(EdgeState::Pending)
    );
    assert_eq!(control.updates.get(), 2);

    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(20));
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Clean);
    assert_eq!(graph.dependencies_of(derived), Ok(vec![replacement_input]));
    assert!(graph.pending_edges().is_empty());
    assert_eq!(control.updates.get(), 3);
}

#[test]
fn missing_dependency_is_recoverable_after_the_rule_provider_repairs_its_handle() {
    let mut graph = AttributeGraph::new();
    let original_input = graph.add_source(ValueStorage::from_i64(10));
    let (rule, control) = controlled_rule(original_input);
    let derived = graph.add_derived(boxed_rule(
        rule,
        update_controlled,
        I64,
        "retargetable pass-through",
    ));

    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(10));
    graph.remove_node(original_input).unwrap();

    assert_eq!(
        graph.read_value(derived),
        Err(GraphError::MissingNode(original_input))
    );
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Dirty);
    assert_eq!(
        graph.debug_cached_value(derived).unwrap().as_i64(),
        Some(10)
    );
    assert_eq!(graph.dependencies_of(derived), Ok(vec![]));

    let replacement_input = graph.add_source(ValueStorage::from_i64(30));
    control.input.set(replacement_input);

    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(30));
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Clean);
    assert_eq!(graph.dependencies_of(derived), Ok(vec![replacement_input]));
    assert_eq!(control.updates.get(), 3);
}

#[test]
fn equal_source_write_is_a_no_op_and_does_not_recompute_dependents() {
    let mut graph = AttributeGraph::new();
    let input = graph.add_source(ValueStorage::from_i64(10));
    let (rule, control) = controlled_rule(input);
    let derived = graph.add_derived(boxed_rule(
        rule,
        update_controlled,
        I64,
        "counted pass-through",
    ));

    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(10));
    assert_eq!(control.updates.get(), 1);

    assert_eq!(
        graph.set_source_value(input, ValueStorage::from_i64(10)),
        Ok(vec![])
    );
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Clean);
    assert!(graph.pending_edges().is_empty());
    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(10));
    assert_eq!(control.updates.get(), 1);

    assert_eq!(
        graph.set_source_value(input, ValueStorage::from_i64(11)),
        Ok(vec![derived])
    );
    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(11));
    assert_eq!(control.updates.get(), 2);
}

#[test]
fn diamond_waits_for_every_pending_branch_before_settling_the_sink() {
    let mut graph = AttributeGraph::new();
    let input = graph.add_source(ValueStorage::from_i64(12));
    let capped = graph.add_derived(boxed_rule(
        CappedRule { input, cap: 10 },
        update_capped,
        I64,
        "min(input, 10)",
    ));
    let passthrough = graph.add_derived(boxed_rule(
        CappedRule { input, cap: 100 },
        update_capped,
        I64,
        "min(input, 100)",
    ));
    let sink_updates = Rc::new(Cell::new(0));
    let sink = graph.add_derived(boxed_rule(
        ProductRule {
            lhs: capped,
            rhs: passthrough,
            updates: Rc::clone(&sink_updates),
        },
        update_product,
        I64,
        "capped * passthrough",
    ));

    assert_eq!(graph.read_value(sink).unwrap().as_i64(), Some(120));
    assert_eq!(sink_updates.get(), 1);

    graph
        .set_source_value(input, ValueStorage::from_i64(13))
        .unwrap();
    assert_eq!(graph.node(capped).unwrap().state(), NodeState::Dirty);
    assert_eq!(graph.node(passthrough).unwrap().state(), NodeState::Dirty);
    assert_eq!(graph.node(sink).unwrap().state(), NodeState::MaybeDirty);
    assert_eq!(graph.edge_state(capped, sink), Ok(EdgeState::Pending));
    assert_eq!(graph.edge_state(passthrough, sink), Ok(EdgeState::Pending));

    let capped_outcome = graph.update_node(capped).unwrap();
    assert!(!capped_outcome.value_changed);
    assert_eq!(graph.node(sink).unwrap().state(), NodeState::MaybeDirty);
    assert_eq!(graph.edge_state(capped, sink), Ok(EdgeState::Settled));
    assert_eq!(graph.edge_state(passthrough, sink), Ok(EdgeState::Pending));
    assert_eq!(sink_updates.get(), 1);

    assert_eq!(graph.read_value(sink).unwrap().as_i64(), Some(130));
    assert_eq!(sink_updates.get(), 2);
    assert_eq!(graph.node(capped).unwrap().state(), NodeState::Clean);
    assert_eq!(graph.node(passthrough).unwrap().state(), NodeState::Clean);
    assert_eq!(graph.node(sink).unwrap().state(), NodeState::Clean);
    assert!(graph.pending_edges().is_empty());
}

#[test]
fn cross_graph_failures_do_not_mutate_either_graph() {
    let mut first = AttributeGraph::new();
    let foreign = first.add_static_attribute(10_i64);

    let mut second = AttributeGraph::new();
    let local = second.add_source(ValueStorage::from_i64(20));
    let (rule, _) = controlled_rule(local);
    let derived = second.add_derived(boxed_rule(
        rule,
        update_controlled,
        I64,
        "local pass-through",
    ));
    assert_eq!(second.read_value(derived).unwrap().as_i64(), Some(20));

    let error = GraphError::GraphMismatch {
        expected: second.id(),
        actual: first.id(),
    };
    assert_eq!(second.read(foreign), Err(error.clone()));
    assert_eq!(
        second.replace_dependencies(derived, [foreign.id()]),
        Err(error)
    );

    assert_eq!(first.read(foreign), Ok(10));
    assert_eq!(second.read_value(derived).unwrap().as_i64(), Some(20));
    assert_eq!(second.dependencies_of(derived), Ok(vec![local]));
    assert_eq!(second.dependents_of(local), Ok(vec![derived]));
    assert_eq!(second.node_count(), 2);
    assert_eq!(second.edge_count(), 1);
}

#[test]
fn destroy_callback_runs_exactly_once_when_each_rule_owner_is_dropped() {
    let drops = Rc::new(Cell::new(0));
    let mut graph = AttributeGraph::new();
    let removed_id = graph.add_derived(boxed_rule(
        DropTrackedRule {
            drops: Rc::clone(&drops),
            value: 1,
        },
        update_drop_tracked,
        I64,
        "removed rule",
    ));

    let removed_node = graph.remove_node(removed_id).unwrap();
    assert_eq!(
        drops.get(),
        0,
        "remove_node transfers ownership to its caller"
    );
    drop(removed_node);
    assert_eq!(drops.get(), 1);

    graph.add_derived(boxed_rule(
        DropTrackedRule {
            drops: Rc::clone(&drops),
            value: 2,
        },
        update_drop_tracked,
        I64,
        "graph-owned rule",
    ));
    drop(graph);

    assert_eq!(drops.get(), 2);
}
