use std::collections::BTreeMap;
use std::fmt::Write;

use attribute_graph::{Edge, EdgeState, NodeId, NodeKind, NodeState};

use crate::{DiffSession, GraphChange, GraphDiff, GraphSnapshot, NodeSnapshot, ValueSummary};

pub fn render_text_snapshot(snapshot: &GraphSnapshot) -> String {
    let mut output = String::new();
    writeln!(&mut output, "Snapshot: {}", snapshot.label).unwrap();
    writeln!(&mut output, "Nodes:").unwrap();

    if snapshot.nodes.is_empty() {
        writeln!(&mut output, "- none").unwrap();
    }

    for node in snapshot.nodes.values() {
        writeln!(&mut output, "- {}", render_node_line(node)).unwrap();
    }

    writeln!(&mut output, "Edges:").unwrap();
    if snapshot.edges.is_empty() {
        writeln!(&mut output, "- none").unwrap();
    }

    for (edge, state) in &snapshot.edges {
        writeln!(
            &mut output,
            "- {} {}",
            render_edge(*edge, &snapshot_node_labels(snapshot)),
            edge_state_name(*state)
        )
        .unwrap();
    }

    output
}

pub fn render_text_diff(diff: &GraphDiff) -> String {
    let mut output = String::new();
    writeln!(
        &mut output,
        "Diff: {} -> {}",
        diff.before_label, diff.after_label
    )
    .unwrap();

    if diff.changes.is_empty() {
        writeln!(&mut output, "- no changes").unwrap();
        return output;
    }

    for change in &diff.changes {
        writeln!(
            &mut output,
            "- {}",
            render_change(change, &diff.node_labels)
        )
        .unwrap();
    }

    output
}

pub fn render_text_timeline(session: &DiffSession) -> String {
    let mut output = String::new();

    for (index, snapshot) in session.snapshots().iter().enumerate() {
        if index == 0 {
            output.push_str(&render_text_snapshot(snapshot));
        } else if let Some(diff) = session.diffs().get(index - 1) {
            output.push_str(&render_text_diff(diff));
        }

        if index + 1 < session.snapshots().len() {
            output.push('\n');
        }
    }

    output
}

pub fn render_mermaid_snapshot(snapshot: &GraphSnapshot) -> String {
    let mut output = String::new();
    writeln!(&mut output, "flowchart LR").unwrap();
    render_mermaid_snapshot_body(&mut output, snapshot, "  ", "");
    write_mermaid_class_defs(&mut output);

    output
}

pub fn render_mermaid_timeline(session: &DiffSession) -> String {
    let mut output = String::new();
    writeln!(&mut output, "flowchart LR").unwrap();
    let mut anchors = Vec::new();

    for (index, snapshot) in session.snapshots().iter().enumerate() {
        let prefix = format!("s{index}_");
        let anchor = format!("{prefix}timeline_anchor");
        anchors.push(anchor.clone());
        writeln!(
            &mut output,
            "  subgraph snapshot{index}[\"{}\"]",
            mermaid_escape(&snapshot.label)
        )
        .unwrap();
        writeln!(&mut output, "    direction TB").unwrap();
        writeln!(&mut output, "    {anchor}[\" \"]").unwrap();
        writeln!(&mut output, "    class {anchor} timeline;").unwrap();
        render_mermaid_snapshot_body(&mut output, snapshot, "    ", &prefix);
        writeln!(&mut output, "  end").unwrap();
    }

    for pair in anchors.windows(2) {
        writeln!(&mut output, "  {} ~~~ {}", pair[0], pair[1]).unwrap();
    }

    write_mermaid_class_defs(&mut output);
    output
}

pub fn render_dot_snapshot(snapshot: &GraphSnapshot) -> String {
    let mut output = String::new();
    writeln!(&mut output, "digraph AttributeGraph {{").unwrap();
    writeln!(&mut output, "  rankdir=LR;").unwrap();
    render_dot_snapshot_body(&mut output, snapshot, "  ", "");
    writeln!(&mut output, "}}").unwrap();
    output
}

pub fn render_dot_timeline(session: &DiffSession) -> String {
    let mut output = String::new();
    writeln!(&mut output, "digraph AttributeGraphTimeline {{").unwrap();
    writeln!(&mut output, "  rankdir=LR;").unwrap();

    for (index, snapshot) in session.snapshots().iter().enumerate() {
        let prefix = format!("s{index}_");
        writeln!(&mut output, "  subgraph cluster_{index} {{").unwrap();
        writeln!(
            &mut output,
            "    label=\"{}\";",
            dot_escape(&snapshot.label)
        )
        .unwrap();
        render_dot_snapshot_body(&mut output, snapshot, "    ", &prefix);
        writeln!(&mut output, "  }}").unwrap();
    }

    writeln!(&mut output, "}}").unwrap();
    output
}

fn render_mermaid_snapshot_body(
    output: &mut String,
    snapshot: &GraphSnapshot,
    indent: &str,
    node_prefix: &str,
) {
    if snapshot.nodes.is_empty() {
        writeln!(output, "{indent}{node_prefix}empty[\"empty graph\"]").unwrap();
    }

    for node in snapshot.nodes.values() {
        writeln!(
            output,
            "{indent}{}[\"{}\"]",
            node_ref(node_prefix, node.id),
            mermaid_escape(&render_node_label(node, "<br/>"))
        )
        .unwrap();
        writeln!(
            output,
            "{indent}class {} {};",
            node_ref(node_prefix, node.id),
            state_class(node.state)
        )
        .unwrap();
    }

    for (edge, state) in &snapshot.edges {
        writeln!(
            output,
            "{indent}{} -->|\"{}\"| {}",
            node_ref(node_prefix, edge.dependent),
            edge_state_name(*state),
            node_ref(node_prefix, edge.dependency)
        )
        .unwrap();
    }
}

fn write_mermaid_class_defs(output: &mut String) {
    writeln!(
        output,
        "  classDef clean fill:#e8f5e9,stroke:#2e7d32,color:#1b1f23;"
    )
    .unwrap();
    writeln!(
        output,
        "  classDef dirty fill:#ffebee,stroke:#c62828,color:#1b1f23;"
    )
    .unwrap();
    writeln!(
        output,
        "  classDef maybe fill:#fff8e1,stroke:#f9a825,color:#1b1f23;"
    )
    .unwrap();
    writeln!(
        output,
        "  classDef timeline fill:transparent,stroke:transparent,color:transparent;"
    )
    .unwrap();
}

fn render_dot_snapshot_body(
    output: &mut String,
    snapshot: &GraphSnapshot,
    indent: &str,
    node_prefix: &str,
) {
    for node in snapshot.nodes.values() {
        writeln!(
            output,
            "{indent}{} [label=\"{}\", style=filled, fillcolor=\"{}\"] ;",
            node_ref(node_prefix, node.id),
            dot_escape(&render_node_label(node, "\\n")),
            dot_fill(node.state)
        )
        .unwrap();
    }

    for (edge, state) in &snapshot.edges {
        writeln!(
            output,
            "{indent}{} -> {} [label=\"{}\"] ;",
            node_ref(node_prefix, edge.dependent),
            node_ref(node_prefix, edge.dependency),
            edge_state_name(*state)
        )
        .unwrap();
    }
}

fn render_change(change: &GraphChange, node_labels: &BTreeMap<NodeId, String>) -> String {
    match change {
        GraphChange::NodeAdded(node) => {
            format!("node {} added ({})", node_name(node), node_kind(node))
        }
        GraphChange::NodeRemoved(node) => {
            format!("node {} removed ({})", node_name(node), node_kind(node))
        }
        GraphChange::NodeStateChanged { id, before, after } => format!(
            "node {} state {} -> {}",
            labeled_id(*id, node_labels),
            state_name(*before),
            state_name(*after)
        ),
        GraphChange::NodeValueChanged { id, before, after } => format!(
            "node {} value {} -> {}",
            labeled_id(*id, node_labels),
            render_optional_value(before.as_ref()),
            render_optional_value(after.as_ref())
        ),
        GraphChange::EdgeAdded { edge, state } => {
            format!(
                "edge {} added ({})",
                render_edge(*edge, node_labels),
                edge_state_name(*state)
            )
        }
        GraphChange::EdgeRemoved { edge, state } => {
            format!(
                "edge {} removed ({})",
                render_edge(*edge, node_labels),
                edge_state_name(*state)
            )
        }
        GraphChange::EdgeStateChanged {
            edge,
            before,
            after,
        } => format!(
            "edge {} state {} -> {}",
            render_edge(*edge, node_labels),
            edge_state_name(*before),
            edge_state_name(*after)
        ),
    }
}

fn render_node_line(node: &NodeSnapshot) -> String {
    let mut pieces = vec![
        node_name(node),
        node_kind(node).to_string(),
        state_name(node.state).to_string(),
    ];

    if let Some(value_type) = &node.value_type {
        pieces.push(format!("type={value_type}"));
    }

    if let Some(debug_name) = &node.debug_name {
        pieces.push(format!("rule={debug_name:?}"));
    }

    if let Some(value) = &node.cached_value {
        pieces.push(format!("value={}", render_value(value)));
    }

    pieces.push(format!("deps={}", render_ids(&node.dependencies)));
    pieces.push(format!("dependents={}", render_ids(&node.dependents)));
    pieces.join(" ")
}

fn render_node_label(node: &NodeSnapshot, line_break: &str) -> String {
    let mut label = format!(
        "{}{line_break}{} {}",
        node_name(node),
        node_kind(node),
        state_name(node.state)
    );

    if let Some(debug_name) = &node.debug_name {
        label.push_str(&format!("{line_break}rule: {debug_name}"));
    }

    if let Some(value) = &node.cached_value {
        label.push_str(&format!("{line_break}value: {}", render_value(value)));
    }

    label
}

fn render_edge(edge: Edge, node_labels: &BTreeMap<NodeId, String>) -> String {
    format!(
        "{} depends on {}",
        labeled_id(edge.dependent, node_labels),
        labeled_id(edge.dependency, node_labels)
    )
}

fn render_ids(ids: &[NodeId]) -> String {
    let ids = ids.iter().map(|id| id_name(*id)).collect::<Vec<_>>();
    format!("[{}]", ids.join(", "))
}

fn render_optional_value(value: Option<&ValueSummary>) -> String {
    value
        .map(render_value)
        .unwrap_or_else(|| "<missing>".to_string())
}

fn render_value(value: &ValueSummary) -> String {
    format!("{} ({})", value.rendered, value.value_type)
}

fn id_name(id: NodeId) -> String {
    format!("#{}", id.raw())
}

fn labeled_id(id: NodeId, node_labels: &BTreeMap<NodeId, String>) -> String {
    node_labels
        .get(&id)
        .map(|label| format!("{label} ({})", id_name(id)))
        .unwrap_or_else(|| id_name(id))
}

fn node_name(node: &NodeSnapshot) -> String {
    node.label
        .as_ref()
        .map(|label| format!("{label} ({})", id_name(node.id)))
        .unwrap_or_else(|| id_name(node.id))
}

fn snapshot_node_labels(snapshot: &GraphSnapshot) -> BTreeMap<NodeId, String> {
    snapshot
        .nodes
        .values()
        .filter_map(|node| node.label.as_ref().map(|label| (node.id, label.clone())))
        .collect()
}

fn node_ref(prefix: &str, id: NodeId) -> String {
    format!("{prefix}n{}", id.raw())
}

fn node_kind(node: &NodeSnapshot) -> &'static str {
    match node.kind {
        NodeKind::Source => "Source",
        NodeKind::Derived => "Derived",
    }
}

fn state_name(state: NodeState) -> &'static str {
    match state {
        NodeState::Clean => "Clean",
        NodeState::Dirty => "Dirty",
        NodeState::MaybeDirty => "MaybeDirty",
    }
}

fn state_class(state: NodeState) -> &'static str {
    match state {
        NodeState::Clean => "clean",
        NodeState::Dirty => "dirty",
        NodeState::MaybeDirty => "maybe",
    }
}

fn dot_fill(state: NodeState) -> &'static str {
    match state {
        NodeState::Clean => "#e8f5e9",
        NodeState::Dirty => "#ffebee",
        NodeState::MaybeDirty => "#fff8e1",
    }
}

fn edge_state_name(state: EdgeState) -> &'static str {
    match state {
        EdgeState::Inactive => "Inactive",
        EdgeState::Settled => "Settled",
        EdgeState::Pending => "Pending",
    }
}

fn mermaid_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "#quot;")
}

fn dot_escape(value: &str) -> String {
    value.replace('"', "\\\"")
}
