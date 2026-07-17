use std::any::type_name;
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
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
    Panic,
}

impl NextAction {
    const fn as_i64(self) -> i64 {
        match self {
            Self::Succeed => 0,
            Self::ReturnError => 1,
            Self::OmitOutput => 2,
            Self::Panic => 3,
        }
    }

    fn from_i64(value: i64) -> Self {
        match value {
            0 => Self::Succeed,
            1 => Self::ReturnError,
            2 => Self::OmitOutput,
            3 => Self::Panic,
            _ => panic!("unknown test action {value}"),
        }
    }

    fn storage(self) -> ValueStorage {
        ValueStorage::from_i64(self.as_i64())
    }
}

struct ControlledRule {
    use_replacement: NodeId,
    original_input: NodeId,
    replacement_input: NodeId,
    next_action: NodeId,
    updates: Rc<Cell<usize>>,
}

struct ControlledRuleControl {
    updates: Rc<Cell<usize>>,
}

struct CountedPassThroughRule {
    input: NodeId,
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

    let use_replacement = context
        .read(rule.use_replacement)?
        .as_bool()
        .expect("test selector should be a bool");
    let input = if use_replacement {
        rule.replacement_input
    } else {
        rule.original_input
    };
    let value = context.read(input)?;
    let next_action = context
        .read(rule.next_action)?
        .as_i64()
        .map(NextAction::from_i64)
        .expect("test action should be an i64");

    match next_action {
        NextAction::Succeed => context.set_output(value),
        // The graph propagates callback errors without interpreting them. This
        // sentinel stands in for an error reported by an external rule provider.
        // Setting output first proves that an error still aborts the whole attempt.
        NextAction::ReturnError => {
            context.set_output(value);
            return Err(GraphError::CycleDetected);
        }
        NextAction::OmitOutput => {}
        NextAction::Panic => panic!("intentional callback panic"),
    }

    Ok(())
}

fn update_counted_pass_through(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<CountedPassThroughRule>(handle);
    rule.updates.set(rule.updates.get() + 1);
    let value = context.read(rule.input)?;
    context.set_output(value);
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

fn controlled_rule(
    use_replacement: NodeId,
    original_input: NodeId,
    replacement_input: NodeId,
    next_action: NodeId,
) -> (ControlledRule, ControlledRuleControl) {
    let updates = Rc::new(Cell::new(0));

    let control = ControlledRuleControl {
        updates: Rc::clone(&updates),
    };

    (
        ControlledRule {
            use_replacement,
            original_input,
            replacement_input,
            next_action,
            updates,
        },
        control,
    )
}

#[test]
fn callback_error_does_not_commit_partial_evaluation_and_can_retry() {
    let mut graph = AttributeGraph::new();
    let use_replacement = graph.add_source(ValueStorage::from_bool(false));
    let original_input = graph.add_source(ValueStorage::from_i64(10));
    let replacement_input = graph.add_source(ValueStorage::from_i64(20));
    let next_action = graph.add_source(NextAction::Succeed.storage());
    let (rule, control) = controlled_rule(
        use_replacement,
        original_input,
        replacement_input,
        next_action,
    );
    let derived = graph.add_derived(boxed_rule(
        rule,
        update_controlled,
        I64,
        "controlled pass-through",
    ));

    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(10));
    graph
        .set_source_value(use_replacement, ValueStorage::from_bool(true))
        .unwrap();
    graph
        .set_source_value(next_action, NextAction::ReturnError.storage())
        .unwrap();

    assert_eq!(graph.update_node(derived), Err(GraphError::CycleDetected));
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Dirty);
    assert_eq!(
        graph.debug_cached_value(derived).unwrap().as_i64(),
        Some(10),
        "the old cache stays internal while the node remains dirty",
    );
    assert_eq!(
        graph.dependencies_of(derived),
        Ok(vec![use_replacement, original_input, next_action])
    );
    assert_eq!(graph.dependents_of(replacement_input), Ok(vec![]));
    assert_eq!(
        graph.pending_edges(),
        vec![
            Edge::new(use_replacement, derived),
            Edge::new(next_action, derived),
        ]
    );
    assert_eq!(control.updates.get(), 2);

    graph
        .set_source_value(next_action, NextAction::Succeed.storage())
        .unwrap();
    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(20));
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Clean);
    assert_eq!(
        graph.dependencies_of(derived),
        Ok(vec![use_replacement, replacement_input, next_action])
    );
    assert!(graph.pending_edges().is_empty());
    assert_eq!(control.updates.get(), 3);
}

#[test]
fn missing_output_preserves_previous_state_and_can_retry() {
    let mut graph = AttributeGraph::new();
    let use_replacement = graph.add_source(ValueStorage::from_bool(false));
    let original_input = graph.add_source(ValueStorage::from_i64(10));
    let replacement_input = graph.add_source(ValueStorage::from_i64(20));
    let next_action = graph.add_source(NextAction::Succeed.storage());
    let (rule, control) = controlled_rule(
        use_replacement,
        original_input,
        replacement_input,
        next_action,
    );
    let derived = graph.add_derived(boxed_rule(
        rule,
        update_controlled,
        I64,
        "occasionally omits output",
    ));

    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(10));
    graph
        .set_source_value(use_replacement, ValueStorage::from_bool(true))
        .unwrap();
    graph
        .set_source_value(next_action, NextAction::OmitOutput.storage())
        .unwrap();

    assert_eq!(
        graph.update_node(derived),
        Err(GraphError::MissingOutput(derived))
    );
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Dirty);
    assert_eq!(
        graph.debug_cached_value(derived).unwrap().as_i64(),
        Some(10)
    );
    assert_eq!(
        graph.dependencies_of(derived),
        Ok(vec![use_replacement, original_input, next_action])
    );
    assert_eq!(graph.dependents_of(replacement_input), Ok(vec![]));
    assert_eq!(
        graph.edge_state(use_replacement, derived),
        Ok(EdgeState::Pending)
    );
    assert_eq!(control.updates.get(), 2);

    graph
        .set_source_value(next_action, NextAction::Succeed.storage())
        .unwrap();
    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(20));
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Clean);
    assert_eq!(
        graph.dependencies_of(derived),
        Ok(vec![use_replacement, replacement_input, next_action])
    );
    assert!(graph.pending_edges().is_empty());
    assert_eq!(control.updates.get(), 3);
}

#[test]
fn missing_dependency_is_recoverable_through_an_existing_conditional_branch() {
    let mut graph = AttributeGraph::new();
    let use_replacement = graph.add_source(ValueStorage::from_bool(false));
    let original_input = graph.add_source(ValueStorage::from_i64(10));
    let replacement_input = graph.add_source(ValueStorage::from_i64(30));
    let next_action = graph.add_source(NextAction::Succeed.storage());
    let (rule, control) = controlled_rule(
        use_replacement,
        original_input,
        replacement_input,
        next_action,
    );
    let derived = graph.add_derived(boxed_rule(
        rule,
        update_controlled,
        I64,
        "conditional pass-through",
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
    assert_eq!(
        graph.dependencies_of(derived),
        Ok(vec![use_replacement, next_action])
    );

    graph
        .set_source_value(use_replacement, ValueStorage::from_bool(true))
        .unwrap();

    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(30));
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Clean);
    assert_eq!(
        graph.dependencies_of(derived),
        Ok(vec![use_replacement, replacement_input, next_action])
    );
    assert_eq!(control.updates.get(), 3);
}

#[test]
fn callback_panic_restores_the_evaluation_stack_before_resuming_unwind() {
    let mut graph = AttributeGraph::new();
    let use_replacement = graph.add_source(ValueStorage::from_bool(false));
    let original_input = graph.add_source(ValueStorage::from_i64(10));
    let replacement_input = graph.add_source(ValueStorage::from_i64(20));
    let next_action = graph.add_source(NextAction::Succeed.storage());
    let (rule, control) = controlled_rule(
        use_replacement,
        original_input,
        replacement_input,
        next_action,
    );
    let derived = graph.add_derived(boxed_rule(
        rule,
        update_controlled,
        I64,
        "panic-safe pass-through",
    ));

    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(10));
    graph
        .set_source_value(original_input, ValueStorage::from_i64(11))
        .unwrap();
    graph
        .set_source_value(next_action, NextAction::Panic.storage())
        .unwrap();

    let panic = catch_unwind(AssertUnwindSafe(|| graph.update_node(derived)))
        .expect_err("the callback should resume its panic");
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"intentional callback panic")
    );
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Dirty);
    assert_eq!(
        graph.debug_cached_value(derived).unwrap().as_i64(),
        Some(10)
    );
    assert_eq!(
        graph.dependencies_of(derived),
        Ok(vec![use_replacement, original_input, next_action])
    );
    assert_eq!(
        graph.pending_edges(),
        vec![
            Edge::new(original_input, derived),
            Edge::new(next_action, derived),
        ]
    );
    assert_eq!(control.updates.get(), 2);

    graph
        .set_source_value(next_action, NextAction::Succeed.storage())
        .unwrap();
    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(11));
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Clean);
    assert!(graph.pending_edges().is_empty());
    assert_eq!(control.updates.get(), 3);
}

#[test]
fn equal_source_write_is_a_no_op_and_does_not_recompute_dependents() {
    let mut graph = AttributeGraph::new();
    let input = graph.add_source(ValueStorage::from_i64(10));
    let updates = Rc::new(Cell::new(0));
    let derived = graph.add_derived(boxed_rule(
        CountedPassThroughRule {
            input,
            updates: Rc::clone(&updates),
        },
        update_counted_pass_through,
        I64,
        "counted pass-through",
    ));

    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(10));
    assert_eq!(updates.get(), 1);

    assert_eq!(
        graph.set_source_value(input, ValueStorage::from_i64(10)),
        Ok(vec![])
    );
    assert_eq!(graph.node(derived).unwrap().state(), NodeState::Clean);
    assert!(graph.pending_edges().is_empty());
    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(10));
    assert_eq!(updates.get(), 1);

    assert_eq!(
        graph.set_source_value(input, ValueStorage::from_i64(11)),
        Ok(vec![derived])
    );
    assert_eq!(graph.read_value(derived).unwrap().as_i64(), Some(11));
    assert_eq!(updates.get(), 2);
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
    let use_foreign = second.add_source(ValueStorage::from_bool(false));
    let local = second.add_source(ValueStorage::from_i64(20));
    let next_action = second.add_source(NextAction::Succeed.storage());
    let (rule, control) = controlled_rule(use_foreign, local, foreign.id(), next_action);
    let derived = second.add_derived(boxed_rule(
        rule,
        update_controlled,
        I64,
        "local-or-foreign pass-through",
    ));
    assert_eq!(second.read_value(derived).unwrap().as_i64(), Some(20));
    second
        .set_source_value(use_foreign, ValueStorage::from_bool(true))
        .unwrap();

    let error = GraphError::GraphMismatch {
        expected: second.id(),
        actual: first.id(),
    };
    assert_eq!(second.read(foreign), Err(error.clone()));
    assert_eq!(second.read_value(derived), Err(error));

    assert_eq!(first.read(foreign), Ok(10));
    assert_eq!(
        second.debug_cached_value(derived).unwrap().as_i64(),
        Some(20)
    );
    assert_eq!(second.node(derived).unwrap().state(), NodeState::Dirty);
    assert_eq!(
        second.dependencies_of(derived),
        Ok(vec![use_foreign, local, next_action])
    );
    assert_eq!(second.dependents_of(local), Ok(vec![derived]));
    assert_eq!(
        second.pending_edges(),
        vec![Edge::new(use_foreign, derived)]
    );
    assert_eq!(control.updates.get(), 2);
    assert_eq!(second.node_count(), 4);
    assert_eq!(second.edge_count(), 3);

    second
        .set_source_value(use_foreign, ValueStorage::from_bool(false))
        .unwrap();
    assert_eq!(second.read_value(derived).unwrap().as_i64(), Some(20));
    assert_eq!(second.node(derived).unwrap().state(), NodeState::Clean);
    assert_eq!(
        second.edge_state(use_foreign, derived),
        Ok(EdgeState::Settled)
    );
    assert_eq!(second.edge_state(local, derived), Ok(EdgeState::Settled));
    assert_eq!(
        second.edge_state(next_action, derived),
        Ok(EdgeState::Settled)
    );
    assert!(second.pending_edges().is_empty());
    assert_eq!(control.updates.get(), 3);
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
