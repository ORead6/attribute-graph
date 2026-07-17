use std::any::type_name;

use attribute_graph::{
    Attribute, AttributeGraph, EdgeState, EvaluationContext, GraphError, NodeState, RuleDescriptor,
    RuleHandle, TypeDescriptor, UpdateFn,
};
use attribute_graph_diff::{
    DiffSession, GraphChange, render_dot_snapshot, render_mermaid_snapshot,
    render_mermaid_timeline, render_text_diff, render_text_snapshot, render_text_timeline,
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
    assert!(text.contains("total (#2) depends on lhs (#0)"));

    let mermaid = render_mermaid_snapshot(session.latest_snapshot().unwrap());
    assert!(mermaid.contains("Pending"));
    assert!(mermaid.contains("n0 -->|\"Pending\"| n2"));
    assert!(mermaid.contains("Dirty"));
}

#[test]
fn captures_and_renders_nested_swiftui_view_scopes_and_their_removal() {
    let mut graph = AttributeGraph::new();
    let mut session = DiffSession::new();

    let settings_screen = graph.create_subgraph(None).unwrap();
    let account_row = graph.create_subgraph(Some(settings_screen)).unwrap();
    let empty_overlay = graph.create_subgraph(Some(settings_screen)).unwrap();
    let screen_padding = graph
        .with_subgraph(settings_screen, |graph| graph.add_static_attribute(16_i64))
        .unwrap();
    let row_height = graph
        .with_subgraph(account_row, |graph| graph.add_static_attribute(44_i64))
        .unwrap();

    session.label_subgraph(settings_screen, "SettingsScreen");
    session.label_subgraph(account_row, "AccountRow");
    session.label_subgraph(empty_overlay, "LoadingOverlay");
    session.label_attribute(screen_padding.attribute(), "SettingsScreen.padding");
    session.label_attribute(row_height.attribute(), "AccountRow.height");
    session.capture("mounted view tree", &graph).unwrap();

    let snapshot = session.latest_snapshot().unwrap();
    assert_eq!(snapshot.subgraphs.len(), 3);
    assert_eq!(
        snapshot.subgraphs[&settings_screen].children,
        vec![account_row, empty_overlay]
    );
    assert_eq!(
        snapshot.subgraphs[&account_row].parent,
        Some(settings_screen)
    );
    assert_eq!(snapshot.subgraphs[&empty_overlay].nodes, vec![]);
    assert_eq!(
        snapshot.node(screen_padding.id()).unwrap().subgraph_id,
        Some(settings_screen)
    );
    assert_eq!(
        snapshot.node(row_height.id()).unwrap().subgraph_id,
        Some(account_row)
    );

    let text = render_text_snapshot(snapshot);
    assert!(text.contains("Subgraphs:"));
    assert!(text.contains(&format!("SettingsScreen ({settings_screen}) parent=<root>")));
    assert!(text.contains(&format!(
        "AccountRow ({account_row}) parent=SettingsScreen ({settings_screen})"
    )));
    assert!(text.contains(&format!(
        "LoadingOverlay ({empty_overlay}) parent=SettingsScreen ({settings_screen}) children=[] nodes=[]"
    )));
    assert!(text.contains(&format!(
        "SettingsScreen.padding (#0) Source Clean subgraph=SettingsScreen ({settings_screen})"
    )));

    let mermaid = render_mermaid_snapshot(snapshot);
    assert!(mermaid.contains(&format!(
        "  subgraph sg0[\"SettingsScreen ({settings_screen})\"]\n    direction TB\n    n0["
    )));
    assert!(mermaid.contains(&format!(
        "    subgraph sg1[\"AccountRow ({account_row})\"]\n      direction TB\n      n1["
    )));
    assert!(mermaid.contains(&format!(
        "    subgraph sg2[\"LoadingOverlay ({empty_overlay})\"]\n      direction TB\n      sg2_empty[\"no attributes\"]"
    )));

    let dot = render_dot_snapshot(snapshot);
    assert!(dot.contains(&format!(
        "  subgraph cluster_sg0 {{\n    label=\"SettingsScreen ({settings_screen})\";\n    n0 ["
    )));
    assert!(dot.contains(&format!(
        "    subgraph cluster_sg1 {{\n      label=\"AccountRow ({account_row})\";\n      n1 ["
    )));
    assert!(dot.contains(&format!(
        "    subgraph cluster_sg2 {{\n      label=\"LoadingOverlay ({empty_overlay})\";\n      sg2_empty [label=\"no attributes\", shape=plaintext];"
    )));

    graph.remove_subgraph(settings_screen).unwrap();
    session.capture("unmounted view tree", &graph).unwrap();

    let diff = session.latest_diff().unwrap();
    assert_eq!(
        diff.changes
            .iter()
            .filter(|change| matches!(change, GraphChange::SubgraphRemoved(_)))
            .count(),
        3
    );
    assert_eq!(
        diff.changes
            .iter()
            .filter(|change| matches!(change, GraphChange::NodeRemoved(_)))
            .count(),
        2
    );

    let removal_text = render_text_diff(diff);
    assert!(removal_text.contains(&format!(
        "subgraph SettingsScreen ({settings_screen}) removed parent=<root> nodes=[#0]"
    )));
    assert!(removal_text.contains(&format!(
        "subgraph AccountRow ({account_row}) removed parent=SettingsScreen ({settings_screen}) nodes=[#1]"
    )));
    assert!(removal_text.contains(&format!(
        "subgraph LoadingOverlay ({empty_overlay}) removed parent=SettingsScreen ({settings_screen}) nodes=[]"
    )));
}
