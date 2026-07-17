# Attribute Graph Diff

Snapshot-based visual debugging for the parent `attribute_graph` crate.

This crate intentionally does not modify the graph framework. It observes the
graph through public APIs, captures labeled snapshots, diffs adjacent snapshots,
and renders the result as text, Mermaid, or Graphviz DOT.

## One-Shot CLI

Run the built-in graph scenario and print text, Mermaid timeline, and DOT timeline output:

```bash
cargo run --manifest-path diff/Cargo.toml
```

Print only one render format:

```bash
cargo run --manifest-path diff/Cargo.toml -- --format text
cargo run --manifest-path diff/Cargo.toml -- --format mermaid
cargo run --manifest-path diff/Cargo.toml -- --format dot
```

Visualize the SwiftUI-style subgraph lifecycle scenario:

```bash
cargo run --manifest-path diff/Cargo.toml -- --scenario subgraph --format mermaid
```

This scenario uses only two attributes and three snapshots. It mounts an
`AccountRow` inside `SettingsScreen`, resolves the row-height dependency, then
recursively removes the screen and row together. Mermaid and DOT use nested
containers for ownership, and the single dependency arrow points from
`AccountRow.height` to `SettingsScreen.contentHeight`.

Text subgraph additions and removals are deterministic snapshot deltas sorted
by identity. Their printed order does not represent teardown or destroy-callback
execution order.

## Example

```rust
use attribute_graph::AttributeGraph;
use attribute_graph_diff::{DiffSession, render_mermaid_snapshot, render_text_timeline};

let mut graph = AttributeGraph::new();
let mut session = DiffSession::new();

session.capture("empty", &graph)?;

let price = graph.add_static_attribute(10_i64);
session.label_attribute(price.attribute(), "price");
session.capture("added price", &graph)?;

graph.set_static(price, 11)?;
session.capture("changed price", &graph)?;

println!("{}", render_text_timeline(&session));
println!("{}", render_mermaid_snapshot(session.latest_snapshot().unwrap()));
# Ok::<(), attribute_graph::GraphError>(())
```

## Realtime Usage

Use `DiffSession::capture` around operations you care about:

```rust
session.capture("before write", &graph)?;
graph.set_static(price, 11)?;
session.capture("after write", &graph)?;
```

This is not internal event streaming; true streaming would require framework
hooks. Snapshot diffing keeps the core framework untouched while still making
state, value, and edge transitions visible.
