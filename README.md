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
- Every graph has a `GraphId`, and every `NodeId` carries both its owning graph
  id and a compact graph-local node number.
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

## Graph And Node Identity

`AttributeGraph::id()` returns the graph's process-unique `GraphId`. A `NodeId`
contains that graph id plus its graph-local number:

```text
g1:n0
^^ ^^
|  local node number
owning graph
```

`NodeId::raw()` still returns the compact local number used by the visualizer,
while `NodeId::graph_id()` identifies the owner. The complete `NodeId`, not its
raw number alone, is the node's identity.

This prevents handles from silently aliasing nodes in another graph. Operations
that return `Result` report `GraphError::GraphMismatch` when given a foreign
handle, even if both graphs happen to contain a local node `0`. Optional lookup
and removal APIs treat a foreign handle as absent.

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

## Removing Nodes

Removing a node first invalidates anything that may have cached its value:

- direct dependents become `Dirty`;
- further descendants become `MaybeDirty`;
- edges touching the removed node are then detached.

The next read must therefore re-run the affected rule instead of returning its
old cached value. If the rule body still tries to read the removed node, the read
returns `GraphError::MissingNode`. Removal does not rewrite rule bodies or
cascade-delete dependents; the rule provider decides whether to remove those
nodes too or update its rule logic.

## Failure And Recovery Contract

A derived-node evaluation is committed only after its callback returns `Ok`,
sets an output of the declared type, and produces a valid acyclic dependency
set. Until then, the evaluation is provisional.

When a callback returns an error, omits its output, or reads a missing
dependency:

- the error is returned to the caller;
- the node remains `Dirty`, so its next read retries the rule;
- a previously cached value stays internal but is not returned as a fresh value;
- the node's previously committed dependencies remain unchanged;
- dependencies read by the failed attempt are not committed;
- pending input edges are not consumed by the failed attempt.

The rule provider can repair the external condition or retarget a missing
dependency and retry the same node. The graph is not poisoned by a returned
`GraphError`.

This is per-node atomicity, not a graph-wide transaction. If the failing rule
successfully updated a dirty dependency before it failed, that dependency's
completed update remains committed. Callback panics are outside this recovery
contract and must not cross a host or FFI boundary.

Related guarantees:

- writing an equal `Bytewise` source value is a no-op and does not dirty or run
  dependents;
- foreign graph handles are rejected before mutation;
- removing a dependency invalidates its dependent chain before detaching edges;
- a configured destroy callback runs once when its owning `RuleDescriptor` is
  dropped. `remove_node` transfers ownership into the returned `Node`, so cleanup
  happens when that returned node is dropped.

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
