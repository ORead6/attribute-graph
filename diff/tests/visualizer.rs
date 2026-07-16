use std::any::type_name;

use attribute_graph::{
    Attribute, AttributeGraph, EdgeState, EvaluationContext, GraphError, NodeState, RuleDescriptor,
    RuleHandle, TypeDescriptor, UpdateFn,
};
use attribute_graph_diff::{
    DiffSession, GraphChange, render_mermaid_snapshot, render_mermaid_timeline, render_text_diff,
    render_text_timeline,
};

const I64: TypeDescriptor = TypeDescriptor::new("i64");

#[derive(Debug)]
struct SumRule {
    lhs: Attribute<i64>,
    rhs: Attribute<i64>,
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

fn update_sum(handle: RuleHandle, context: &mut EvaluationContext<'_>) -> Result<(), GraphError> {
    let rule = rule_body::<SumRule>(handle);
    let lhs = context.read_attribute(rule.lhs)?;
    let rhs = context.read_attribute(rule.rhs)?;

    context.set_output_value(lhs + rhs);
    Ok(())
}

#[test]
fn captures_node_value_and_edge_changes_as_a_timeline() {
    let mut graph = AttributeGraph::new();
    let mut session = DiffSession::new();

    session.capture("empty", &graph).unwrap();

    let lhs = graph.add_static_attribute(10_i64);
    let rhs = graph.add_static_attribute(2_i64);
    session.label_attribute(lhs.attribute(), "lhs");
    session.label_attribute(rhs.attribute(), "rhs");

    let total = graph
        .add_dynamic_attribute::<i64>(boxed_rule(
            SumRule {
                lhs: lhs.attribute(),
                rhs: rhs.attribute(),
            },
            update_sum,
            I64,
            "lhs + rhs",
        ))
        .unwrap();
    session.label_attribute(total.attribute(), "total");
    session.capture("created attributes", &graph).unwrap();

    let created = session.latest_diff().unwrap();
    assert_eq!(
        created
            .changes
            .iter()
            .filter(|change| matches!(change, GraphChange::NodeAdded(_)))
            .count(),
        3
    );

    assert_eq!(graph.read(total).unwrap(), 12);
    session.capture("evaluated total", &graph).unwrap();

    let evaluated = session.latest_diff().unwrap();
    assert!(evaluated.changes.iter().any(|change| {
        matches!(
            change,
            GraphChange::NodeStateChanged {
                id,
                before: NodeState::Dirty,
                after: NodeState::Clean,
            } if *id == total.id()
        )
    }));
    assert!(evaluated.changes.iter().any(|change| {
        matches!(
            change,
            GraphChange::EdgeAdded {
                state: EdgeState::Settled,
                ..
            }
        )
    }));

    let timeline = render_text_timeline(&session);
    assert!(timeline.contains("Snapshot: empty"));
    assert!(timeline.contains("Diff: created attributes -> evaluated total"));

    let mermaid = render_mermaid_snapshot(session.latest_snapshot().unwrap());
    assert!(mermaid.contains("flowchart LR"));
    assert!(mermaid.contains("Settled"));
    assert!(mermaid.contains("lhs + rhs"));
    assert!(mermaid.contains("total (#2)"));

    let mermaid_timeline = render_mermaid_timeline(&session);
    assert!(mermaid_timeline.contains("flowchart LR"));
    assert!(mermaid_timeline.contains("created attributes"));
    assert!(mermaid_timeline.contains("evaluated total"));
}

#[test]
fn shows_pending_edges_when_a_source_write_dirties_a_dependent() {
    let mut graph = AttributeGraph::new();
    let mut session = DiffSession::new();

    let lhs = graph.add_static_attribute(10_i64);
    let rhs = graph.add_static_attribute(2_i64);
    session.label_attribute(lhs.attribute(), "lhs");
    session.label_attribute(rhs.attribute(), "rhs");

    let total = graph
        .add_dynamic_attribute::<i64>(boxed_rule(
            SumRule {
                lhs: lhs.attribute(),
                rhs: rhs.attribute(),
            },
            update_sum,
            I64,
            "lhs + rhs",
        ))
        .unwrap();
    session.label_attribute(total.attribute(), "total");

    graph.read(total).unwrap();
    session.capture("settled", &graph).unwrap();

    graph.set_static(lhs, 11).unwrap();
    session.capture("lhs changed", &graph).unwrap();

    let diff = session.latest_diff().unwrap();
    assert!(diff.changes.iter().any(|change| {
        matches!(
            change,
            GraphChange::NodeStateChanged {
                id,
                before: NodeState::Clean,
                after: NodeState::Dirty,
            } if *id == total.id()
        )
    }));
    assert!(diff.changes.iter().any(|change| {
        matches!(
            change,
            GraphChange::EdgeStateChanged {
                edge,
                before: EdgeState::Settled,
                after: EdgeState::Pending,
            } if edge.dependency == lhs.id() && edge.dependent == total.id()
        )
    }));

    let text = render_text_diff(diff);
    assert!(text.contains("Pending"));
    assert!(text.contains("lhs (#0) -> total (#2)"));

    let mermaid = render_mermaid_snapshot(session.latest_snapshot().unwrap());
    assert!(mermaid.contains("Pending"));
    assert!(mermaid.contains("Dirty"));
}
