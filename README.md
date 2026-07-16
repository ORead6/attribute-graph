# Attribute Graph

A small Rust starting point for an AttributeGraph-style runtime.

The graph is now shaped so rules can come from outside this layer, such as a
Swift bridge or another host runtime. The graph owns nodes, values, state,
dependency edges, pending edges, and evaluation bookkeeping. Rule providers own
the actual rule logic.

## Core Shape

- Source nodes store externally supplied values.
- Derived nodes store an opaque rule body handle plus an update callback.
- Public callers can use typed `StaticAttribute<T>` and `DynamicAttribute<T>`
  handles instead of passing raw `NodeId`s around.
- Cached values use `ValueStorage`: a type descriptor plus bytes.
- Rust-side values use `AttributeValue` to convert to and from cached storage.
- Rules read dependencies through `EvaluationContext::read`.
- Typed rules can read dependencies through `EvaluationContext::read_attribute`
  and write outputs through `set_output_value`.
- Reads recorded during evaluation replace the derived node's active dependency
  set.
- Rules write their output with `EvaluationContext::set_output`.
- Dirty nodes are recomputed lazily when `update_node` or a dependent read needs
  their value.
- Downstream nodes can be `MaybeDirty`, meaning their dependencies must validate
  before their cached value is trusted.
- External callers should use `read(attribute)` when they want a lazily
  validated typed value.
- Recomputed derived values are compared with the previous cached value before
  downstream dependents are dirtied.

## Derived Rule Model

A derived node does not match on node ids or know its own business logic.

It stores:

```text
RuleDescriptor {
    body: RuleHandle,        // opaque rule body from the provider
    update: UpdateFn,        // callback that knows how to run that body
    body_type: TypeDescriptor,
    value_type: TypeDescriptor,
    debug_name: &'static str,
}
```

When a derived node is dirty, the graph:

1. Finds the stored `RuleDescriptor`.
2. Builds an `EvaluationContext`.
3. Calls `update(body, &mut context)`.
4. Lets the callback read inputs through the context.
5. Replaces active dependencies with the nodes actually read.
6. Validates and compares the callback's output with the old cached value.
7. Stores the new output and clears pending edges into that node.
8. Dirties downstream dependents only if the value meaningfully changed.

## Value Comparison

The first comparison policy is intentionally simple:

```text
ValueComparison::Bytewise
```

That is enough for the current byte-backed values like `bool`, `i64`, and static
strings. A value can also use:

```text
ValueComparison::AlwaysChanged
```

for cases where every recomputation should invalidate downstream dependents.

Longer term, this is where a Swift or other host bridge would supply comparison
callbacks through the type descriptor, so a value can use host semantics such as
Swift `Equatable`, identity, bitwise comparison, or custom "always changed"
behavior.

## Dependency Convention

Edges point from dependency to dependent.

If `total` reads `price`, the graph stores:

```text
price -> total
```

The `Edge` struct says that directly:

```rust
pub struct Edge {
    // `dependent` depends on `dependency`.
    // If `dependency` changes, `dependent` may need to be recomputed.
    pub dependency: NodeId,
    pub dependent: NodeId,
}
```

## Diff Visualizer

The visualizer lives in `diff/` and does not change the graph framework code.

Run the default scenario:

```sh
cargo run --manifest-path diff/Cargo.toml
```

Run a specific scenario and output format:

```sh
cargo run --manifest-path diff/Cargo.toml -- --scenario conditional --format mermaid
```

Available scenarios are `basic`, `same-output`, and `conditional`. Visual
arrows point from the dependent attribute to the dependency it reads, so an
arrow from `grand total` to `total` means "`grand total` depends on `total`";
the framework still stores edges internally as dependency-to-dependent.

## What Is Still Out Of Scope

This pass intentionally does not include lists, `foreach`, subgraphs,
scheduling, animation transactions, or host-supplied comparison callbacks.
