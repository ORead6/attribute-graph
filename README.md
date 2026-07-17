# Attribute Graph

A small pull-based scalar dependency graph in Rust. It stores source and derived
values, discovers dependencies while rules run, invalidates cached values, and
recomputes lazily when a consumer reads an output.

This crate is intended to be the reactive kernel underneath another layer, such
as a Swift bridge or UI runtime. It does not own widgets, collection membership,
render scheduling, gestures, clocks, or transactions. Those layers write source
attributes, decide when to read derived attributes, and apply the returned
values.

## At A Glance

### What it supports

- Typed source attributes with externally supplied values.
- Typed derived attributes backed by externally supplied rule callbacks.
- Runtime dependency discovery through rule reads.
- Conditional dependencies among handles fixed in an immutable rule.
- Nested derived values, caching, and lazy invalidation.
- `Clean`, `Dirty`, and `MaybeDirty` state propagation.
- Bytewise change detection and an `AlwaysChanged` policy.
- Graph-scoped node and subgraph identity, cycle rejection, removal, and inspection.
- Nested ownership regions with scoped construction and recursive teardown.
- Recovery from provisional evaluation failures without poisoning the graph.

### What it expects from the next layer

- Installed rule definitions remain semantically immutable.
- Every changing input that can affect an output or dependency choice is a
  source attribute.
- Rules read inputs only through `EvaluationContext` and set an output of their
  declared type.
- The consumer explicitly reads the outputs it needs; the graph does not push
  values or schedule rendering.
- The consumer decides where ownership regions begin and end, and uses
  two-phase replacement when a rule's fixed handle set changes.
- Opaque rule bodies remain on one owning thread until their cross-thread safety
  contract is encoded in the API.

### What it returns

- Fresh typed values from `read`, or `GraphError`.
- Node-local dependency and invalidation metadata from explicit updates.
- Direct dependents marked or kept dirty by low-level source writes.
- Sorted dependency, edge, pending-edge, and topology data, plus node states.
- Ownership of a detached `Node` from `remove_node`.

The graph does **not** return a render patch, a complete transitive work list, or
an event stream. The next layer decides what constitutes a render root and when
to read it.

## Subgraphs

A subgraph is an ownership and lifetime region inside one `AttributeGraph`, not
a separate evaluation graph. Nodes created in a construction scope belong to
the innermost active scope. Parentage is fixed, and removing a subgraph
atomically and permanently removes its descendants and owned nodes while
invalidating surviving dependents connected by active edges.

IDs are never reused, so stale node handles return `MissingNode`. The graph
cannot discover opaque handles stored in an inactive conditional path; choosing
such a handle after its target is removed also returns `MissingNode`. Destroy
callbacks run only after structural detachment and are release-only. V1 does
not include detach/reinsert, scheduling, rendering, or cross-graph
dependencies. See [Subgraph V1 Contract](docs/subgraphs.md) for the complete
lifetime and failure contract.

## Core Mental Model

There are two kinds of node:

| Node | Meaning | Initial state |
| --- | --- | --- |
| Source / `StaticAttribute<T>` | A value written by the outside world | `Clean`, with a value |
| Derived / `DynamicAttribute<T>` | A value calculated by a rule | `Dirty`, without a value or dependencies |

The normal lifecycle is:

```text
consumer writes a source
        -> direct dependents become Dirty
        -> transitive descendants become MaybeDirty
        -> no rule runs yet

consumer reads a required output
        -> graph validates only the required dependency chain
        -> dirty rules read inputs and calculate outputs
        -> successful reads become the new committed dependency set
        -> changed outputs dirty their direct dependents
        -> consumer receives the fresh value
```

Several source writes can happen before a read. The eventual evaluation sees the
latest values and may run once, but every distinct write still updates the graph
and invalidation state. This is consumer-controlled deferred evaluation, not an
atomic transaction or a scheduler.

## Node State And Lazy Propagation

| State | Meaning | What a read does |
| --- | --- | --- |
| `Clean` | The cache is current | Returns the cache without running the rule |
| `Dirty` | A direct input changed or no cache exists | Runs the rule |
| `MaybeDirty` | A transitive input may have changed | Validates committed dependencies first |

For a chain `A -> B -> C`, changing source `A` makes `B` dirty and `C` maybe
dirty. Reading `C` first validates `B`. If `B` recomputes to the same value, `C`
becomes clean without running its own rule.

`Node::is_dirty()` reports only `Dirty`; callers that inspect state must handle
`MaybeDirty` separately. Application code should normally use `read` rather than
trying to implement validation from node states.

## Dynamic And Conditional Dependencies

Dependencies are discovered during evaluation rather than declared in a
separate public edge API. Each callback starts with an empty provisional read
set. Every `EvaluationContext::read` or `read_attribute` records the node that
was actually read.

For conditional content:

```text
show_details -> visible_content
summary      -> visible_content   when show_details is false
details      -> visible_content   when show_details is true
```

The rule always reads `show_details`, then reads one branch. After a successful
evaluation, the graph commits only the selector and selected branch. Changes to
the inactive branch do not invalidate `visible_content`.

Important details:

- The new successful read set replaces the previous dependency set; it is not
  merged with it.
- Dependency changes commit even when the new output compares equal.
- A failed evaluation keeps the previous committed dependency set.
- A dirty conditional node continues to expose its previous branch edges until
  the next successful evaluation commits a new set.
- Conditional choices may select among handles already stored in the immutable
  rule. Introducing a new handle requires rebuilding the rule and affected
  downstream rules.

## Public API Map

Prefer the typed API in application-facing adapters. The raw `NodeId` and
`ValueStorage` methods expose the same runtime for low-level bridges and rule
providers.

| Task | Typed API | Low-level API | Result or effect |
| --- | --- | --- | --- |
| Create a source | `add_static_attribute<T>` | `add_source` | A clean source handle |
| Create a derived value | `add_dynamic_attribute<T>` | `add_derived` | A dirty derived handle; typed creation checks the declared output type |
| Write external state | `set_static` | `set_source_value` | Stores the source and marks stale dependents; does not evaluate |
| Read a fresh value | `read` | `read_value` | Lazily validates and returns a value or `GraphError` |
| Explicitly validate/update | `update_dynamic` | `update_node` | Returns a node-local `UpdateOutcome` |
| Remove a node | — | `remove_node` | Invalidates dependents, detaches edges, and returns `Option<Node>` |
| Build an ownership region | — | `create_subgraph`, `with_subgraph`, or `build_subgraph` | Automatically owns nodes created in the active scope |
| Remove an ownership region | — | `remove_subgraph` | Recursively detaches descendants and returns a `SubgraphRemoval` summary |
| Inspect the graph | handle `.id()` | `node`, `dependencies_of`, `edges`, and related methods | Current committed graph metadata |

### Source writes

`set_static` is the simple typed entry point and returns `Result<(), GraphError>`.
The low-level `set_source_value` returns `Vec<NodeId>` containing the source's
**direct** dependents that were marked or kept `Dirty`. A returned node may have
already been dirty from an earlier write, so this is not a state-transition
notification. Transitive descendants become `MaybeDirty`, but they are not
included in that vector.

An equal `Bytewise` source write is a no-op and returns an empty vector. A value
using `AlwaysChanged` invalidates on every write.

`mark_changed(node)` is an advanced manual invalidation operation. It marks that
node's dependents stale; it does not dirty, recompute, or change the node itself.
Most integrations should write source values instead.

### Reads

`read(attribute)` and `read_value(node)` are the freshness boundary. They may
recursively evaluate dirty dependencies before returning. A read returns the
value and discards update metadata.

Do not render from `debug_cached_value`: a dirty node may still retain its old
cache internally for comparison and recovery. That cache is not considered a
fresh public value.

### Explicit updates and `UpdateOutcome`

`update_dynamic` and `update_node` return:

```text
UpdateOutcome {
    dependency_changes: {
        added:    [...],
        removed:  [...],
        retained: [...],
    },
    value_changed: bool,
    dirtied_dependents: [...],
}
```

- `dependency_changes` describes only the requested node's latest
  recomputation. Its vectors are sorted.
- `value_changed` compares that node's old and new output. Its first successful
  output is changed because no old value exists.
- `dirtied_dependents` contains direct downstream nodes marked or kept `Dirty`
  when the output meaningfully changed. It does not mean each node newly
  transitioned from clean.
- Nested dependency updates are not accumulated into the parent's outcome.
- A default, empty outcome can mean the node was already clean or that a
  `MaybeDirty` path validated as unchanged without running this node's callback.

An explicit update therefore must not be treated as a graph-wide change set or
render patch.

If you are integrating a consumer now, continue with
[Plugging In The Next Layer](#plugging-in-the-next-layer). The intervening
sections define the lower-level provider, value, lifetime, and failure contracts
that adapter must uphold.

## Rule Provider Contract

A derived node stores a descriptor supplied by a rule provider:

```text
RuleDescriptor {
    body: RuleHandle,        // opaque provider-supplied representation
    update: UpdateFn,        // callback that understands that body
    destroy: Option<DestroyFn>,
    body_type: TypeDescriptor,
    value_type: TypeDescriptor,
    debug_name: &'static str,
}
```

The graph owns the installed `RuleDescriptor`, but it does not understand the
body or business logic. A provider must uphold these rules:

1. Keep the rule's callback, fixed dependency handles, constants affecting
   output, and evaluation meaning semantically immutable.
2. Put every runtime-changing evaluation input in a source attribute.
3. Read graph values through `EvaluationContext`; bypassed reads are invisible
   to dependency tracking.
4. Return `Ok(())` and call `set_output` or `set_output_value` with the declared
   output type. If a callback sets multiple outputs, the last one wins, so
   providers should set exactly one.
5. Keep callbacks computational. Apply UI side effects only after a successful
   public read or update.
6. Never retain an `EvaluationContext` after the callback returns.
7. Keep opaque body, update, and destroy types matched, and uphold the selected
   ownership mode described below.
8. Ensure destroy callbacks do not panic and host/FFI boundaries do not allow a
   Rust panic to cross them.

The current low-level `RuleHandle` is external-provider plumbing rather than a
safe application rule-authoring API. `destroy: None` means the provider retains
ownership and guarantees the handle remains valid. `destroy: Some` gives that
descriptor one release/destruction responsibility. A reference-counted object
may have several separately retained descriptors, but each retain unit must be
released exactly once.

Treat erased bodies as one-thread-owned until `Send`/`Sync` requirements are
encoded. Rust auto-traits on the surrounding graph do not prove the hidden body
is thread-safe.

`add_dynamic_attribute` consumes its `RuleDescriptor`. If its declared output
type does not match `T`, it returns `RuleValueTypeMismatch` and immediately drops
the descriptor, including running any configured destroy callback. A bridge
must not release that ownership unit a second time.

`UpdateFn` currently returns the closed `GraphError` enum. There is no dedicated
provider or domain-error payload yet, so a bridge must define how its callback
failures map into the current error model.

### Changing a rule

There is no in-place rule replacement. The replacement gets a new `NodeId`, so
every downstream immutable rule holding the old ID must also be replaced.

Use a two-phase swap:

1. Build replacement nodes from inputs toward downstream roots, because each new
   rule needs the IDs of its new dependencies.
2. Switch the consumer to the replacement root.
3. Remove the old chain from downstream dependents toward upstream inputs.

For collection membership, replace membership-dependent aggregates and their
downstream owners before removing a row input they still reference.

## Values, Types, And Comparison

`ValueStorage` is a type descriptor plus bytes and a comparison policy. The
built-in typed `AttributeValue` implementations are `bool`, `i64`, and `String`.
Low-level storage also has a static-string helper.

`TypeDescriptor` currently identifies a format by a public `&'static str` name.
Providers must choose unique names and keep each name tied to one byte format;
the runtime cannot detect two incompatible formats using the same name.

Custom `AttributeValue` implementations must:

- return storage whose descriptor matches `type_descriptor()`;
- encode bytes that their `from_storage` implementation can decode;
- use a stable, canonical representation if bytewise equality matters.

The graph validates an output's descriptor before committing it. It cannot prove
that same-tag bytes are decodable. Malformed bytes may therefore commit on a
`Clean` node; later typed reads return `ValueDecodeFailed` repeatedly and do not
retry the callback. Repair requires replacing the malformed source value or
rebuilding/invalidating the responsible derived path outside the current API.

Current comparison policies are:

- `Bytewise`: when both old and new policies are `Bytewise`, equal descriptors
  and bytes mean unchanged;
- `AlwaysChanged`: if either old or new policy is `AlwaysChanged`, the value is
  considered changed.

An unchanged derived result clears the relevant pending state and prevents
unnecessary downstream callbacks. Host-supplied comparators are not implemented.

`ValueStorage` is runtime cache storage, not a persistence or wire format. For
example, the current raw `i64` representation uses native-endian bytes.

## Identity And Lifetime

Every graph receives a process-runtime `GraphId`. Every node has a complete
identity of `(GraphId, graph-local number)`, displayed as `g1:n0`.

- Store and compare the complete `NodeId`.
- `NodeId::raw()` is only a compact graph-local label.
- IDs are not reused within a graph, but they are not persistent or
  cross-process identifiers.
- Result-returning operations reject foreign handles with `GraphMismatch`.
- Optional lookups and `remove_node` treat missing or foreign handles as absent
  and return `None`; `remove_subgraph` is result-returning and reports the
  corresponding error.

### Removing nodes

`remove_node` first invalidates the dependent chain, then detaches active and
pending edges touching the node. Removal does not cascade-delete dependents and
does not rewrite their immutable rule bodies.

A dependent no longer lists the removed node in its committed dependency set,
but its callback may still hold the removed `NodeId`. Its next evaluation then
returns `MissingNode` unless an existing conditional branch avoids that handle.

When a derived node is removed, the returned `Node` owns its `RuleDescriptor`.
A configured destroy callback runs when that returned node is dropped, not
necessarily at the moment `remove_node` is called. A removed source node has no
rule. Dropping the graph drops all descriptors it still owns.

For individual removal, the next layer must still choose safe lifetime order.
Use a subgraph when a group and its nested children should instead have one
recursive lifetime boundary.

## Failure And Recovery

A derived-node attempt is provisional until:

- the callback returns `Ok`;
- it sets an output with the declared descriptor;
- every dependency exists in the same graph;
- the resulting dependency set is acyclic.

If the callback returns an error, omits output, reads a missing dependency,
produces a wrong descriptor, or encounters a dependency error:

- the requested node remains stale and retryable rather than being incorrectly
  marked clean;
- a node whose callback runs and fails remains `Dirty`, including a
  `MaybeDirty` node promoted to `Dirty` by a changed dependency;
- a `MaybeDirty` node can remain `MaybeDirty` when dependency validation fails
  before its own callback runs;
- the failed callback's provisional output and read set are discarded;
- the attempt does not replace the dependency set that existed when it began;
- the previous cache stays internal and is not returned as fresh.

Recovery is atomic only for the requested node. A dirty derived dependency that
successfully updated during a nested read remains committed even if its parent
later fails. Earlier dependencies may also validate successfully and settle
their pending edges before a later dependency fails. A failed node does not run
its normal successful cleanup, but this nested work is not rolled back.

An explicit `remove_node` is independent of an evaluation attempt: it has already
detached its edge before a later callback reports `MissingNode`.

`ValueDecodeFailed` is different: it can be discovered after same-tag malformed
bytes were already committed on a clean node. It does not automatically make the
node retry its callback; see [Values, Types, And Comparison](#values-types-and-comparison).

Under an unwinding panic strategy, the graph removes the active evaluation frame
and resumes the same panic. A caller that catches it can continue using the
graph, but the panic is not converted to `GraphError` or rolled back. With
`panic=abort`, the process terminates. Panics must not cross a host or FFI
boundary.

## Plugging In The Next Layer

The intended ownership boundary is:

| Attribute graph owns | Consumer / UI / provider layer owns |
| --- | --- |
| Scalar nodes, cached values, and state | Model-to-attribute registries |
| Dependency discovery and committed edges | Collection keys, order, membership, and cell reuse |
| Lazy validation and value comparison | Scheduling and deciding when to read/render |
| Per-node update metadata and recovery | Render patches, widgets, and platform view mutation |
| Edge and storage cleanup plus scoped group teardown | Choosing ownership regions and downstream rebuild order |
| Graph identity and cycle checks | Gesture recognition, clocks, easing, and transactions |

A practical integration loop is:

1. Create one owning adapter around `AttributeGraph`.
2. Store typed attribute handles beside the model, component, or view state that
   owns them.
3. Convert model changes, events, gesture output, and animation ticks into source
   writes.
4. Schedule work outside the graph. Do not treat direct dependent IDs or pending
   edges as a complete render queue.
5. During a render pass, read only the derived outputs required by visible or
   otherwise active UI.
6. Apply returned values to platform views only after reads succeed.
7. Keep keyed rows' existing node handles across reorder. On membership changes,
   build any replacement chain from inputs toward its root, switch the consumer
   root, then retire the old chain from downstream dependents toward inputs.
8. Handle `GraphError` at the adapter boundary and decide whether to retry,
   rebuild, or surface a host error.

Illustrative pseudocode for a typed adapter looks like this (`rule_provider` and
`renderer` are consumer-owned objects, not crate APIs):

```rust
let mut graph = AttributeGraph::new();
let price = graph.add_static_attribute(10_i64);
let quantity = graph.add_static_attribute(2_i64);

// `make_total_rule` belongs to your Rust/Swift rule-provider layer and returns
// a RuleDescriptor whose callback reads price and quantity.
let total = graph.add_dynamic_attribute::<i64>(
    rule_provider.make_total_rule(price.attribute(), quantity.attribute()),
)?;

// An event updates source state. No derived rule runs yet.
graph.set_static(price, 12)?;

// The render layer chooses when to pull the fresh output.
let value = graph.read(total)?;
renderer.set_total(value);
```

The adapter owns `rule_provider`, `renderer`, scheduling, and error handling.
`AttributeGraph` only owns the reactive value and dependency mechanics.

See [UI Consumer Contracts](docs/ui-consumer-contracts.md) and the executable
[`ui_consumer_contracts` tests](tests/ui_consumer_contracts.rs) for conditional
content, keyed `foreach`, table aggregates, list-view cell reuse, animations,
gestures, and cleanup behavior.

## Inspection And Diagnostics

Edges are stored from dependency to dependent:

```text
price -> total
```

Useful inspection methods include:

- `node`, `node_count`, `edge_count`, and `contains_node`;
- `dependencies_of` and `dependents_of`;
- `edges`, `pending_edges`, and `edge_state`;
- `topological_order`.

These describe the **last successfully committed** dependency graph:

- a new unevaluated derived node appears dependency-free;
- a failed evaluation leaves its prior edges intact;
- a dirty conditional node keeps its prior branch edges until success;
- inspection vectors and edges are sorted for deterministic output.

Pending edges mean a previously committed relationship still needs lazy
validation after an upstream change. They are bookkeeping, not scheduled jobs or
an executable work queue.

`edge_state` reports:

- `Inactive` when no committed edge exists between the two nodes;
- `Settled` when the committed edge has consumed the current upstream value;
- `Pending` when the committed edge still requires lazy validation.

`topological_order` orders the last committed edge snapshot. It does not evaluate
or validate nodes first.

The diff visualizer in `diff/` renders graph snapshots and transitions. Run:

```sh
cargo run --manifest-path diff/Cargo.toml
```

Or choose a scenario and output format:

```sh
cargo run --manifest-path diff/Cargo.toml -- --scenario conditional --format mermaid
cargo run --manifest-path diff/Cargo.toml -- --scenario subgraph --format mermaid
```

Available scenarios are `basic`, `same-output`, `conditional`, and `subgraph`.
The SwiftUI-style `subgraph` scenario renders nested `SettingsScreen` and
`AccountRow` ownership scopes, one cross-scope layout dependency, and recursive
teardown across three snapshots. Mermaid and DOT group owned attributes inside
labeled subgraph containers. The arrow points from `AccountRow.height` to the
dependent `SettingsScreen.contentHeight`, matching the framework's internal
dependency-to-dependent edge order.

## Explicit Non-Goals

The fundamental graph currently does not provide:

- native lists, `foreach`, ordering, or keyed diffing;
- subgraph detach/reinsert, suspension, scheduling, or rendering semantics;
- an observer/push system, scheduler, renderer, or render-patch format;
- batching, graph-wide transactions, frame transactions, or animation
  transactions;
- gesture recognition, clocks, easing, or platform event handling;
- in-place rule replacement or identity-preserving rule mutation;
- a custom host comparison callback;
- a persistence or portable serialization format;
- a dedicated provider/domain-error payload;
- an encoded cross-thread safety contract for opaque rule bodies.

Downstream layers can coordinate several of these concepts using the scalar
graph, as the UI consumer contract tests demonstrate, but they remain separate
responsibilities.

## Source Layout

`src/lib.rs` is only the public facade. The implementation is grouped into
private modules, and the existing crate-root API is re-exported unchanged:

- `attribute.rs`: typed static, dynamic, and erased attribute handles;
- `identity.rs`: graph, node, and subgraph identities plus graph-id allocation;
- `value.rs`: type descriptors, value storage, comparison, and typed codecs;
- `rule.rs`: opaque rule handles, callbacks, descriptors, and destruction;
- `node.rs`: node kind, state, cached storage, and dependency ownership;
- `dependency.rs`: edges, edge states, dependency changes, and update outcomes;
- `error.rs`: the graph error contract;
- `subgraph.rs`: ownership hierarchy inspection and removal summaries;
- `graph/subgraphs.rs`: scoped construction and recursive teardown algorithms;
- `graph.rs`: graph storage, lazy evaluation, invalidation, inspection, and
  evaluation context behavior.

This keeps consumer imports such as `attribute_graph::AttributeGraph` stable
without making the internal file layout part of the public API.

## Running The Project

Run the complete graph test suite:

```sh
cargo test --all-targets
```

Run only the UI consumer contracts:

```sh
cargo test --test ui_consumer_contracts
```

Run only the subgraph lifecycle contracts:

```sh
cargo test --test subgraphs
```

Run the visualizer tests:

```sh
cargo test --manifest-path diff/Cargo.toml --all-targets
```

Run lint checks:

```sh
cargo clippy --all-targets -- -D warnings
cargo clippy --manifest-path diff/Cargo.toml --all-targets -- -D warnings
```
