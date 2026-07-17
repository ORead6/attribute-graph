# UI Consumer Contracts

The scenarios in `tests/ui_consumer_contracts.rs` show how a UI-oriented layer
can use `AttributeGraph` as its reactive kernel. They do not add widgets,
collections, rendering, clocks, or input recognition to the graph itself.

The shared flow is:

```text
UI adapter writes source attributes
        -> graph marks affected derived attributes stale
        -> renderer reads the outputs it needs
        -> graph evaluates lazily and records the dependencies actually read
        -> renderer applies the returned values to platform views
```

## Responsibility Boundary

| Scenario | UI consumer owns | Attribute graph proves |
| --- | --- | --- |
| Conditional content | The selector and fixed branch handles | Only the selected branch becomes an active dependency |
| Keyed `foreach` | Membership, order, stable keys, per-item node registry, and cleanup | Existing nodes retain their IDs and per-item values update independently |
| Table | Row membership and rebuilding membership-dependent aggregate rules | Cell changes propagate through the affected row and aggregate only |
| List view | Viewport/selection state and cell reuse | A reused cell depends only on the row it currently reads |
| Animation | Clock, duration, easing, batching render reads, and transaction metadata | A read evaluates from the latest progress value and compares the result |
| Gesture | Recognition, event ordering, cancellation, and coordinate conversion | Gesture sources feed deterministic conditional derived values |

## Executable Scenarios

### Conditional content

The consumer supplies `show_details`, `summary`, and `details` sources. The
visible-content rule always reads the selector and then reads one branch. An
inactive content change does not invalidate the visible output. Switching the
selector replaces the old content edge with the newly selected edge.

### Keyed `foreach` and child lifetime

The mock `KeyedForEachAdapter` holds a key-to-row map and a separate order list.
The adapter—not the graph—uses those keys to retain each row's price, quantity,
and subtotal nodes. Reordering keys therefore preserves all three node IDs.
Editing one model dirties only its subtotal. Deleting a key makes the adapter
remove all nodes owned by that row, while inserting a key creates fresh node
IDs. Duplicate keys are rejected before the adapter mutates the graph.

This is a consumer-owned node group: the higher layer decides its lifetime,
calls `remove_node`, and drops the returned `Node`. The fundamental graph
detaches edges and removes storage; for a derived node with a configured destroy
callback, dropping the returned node then runs that callback.

### Tables and membership-dependent rules

A row subtotal is an immutable rule over that row's two cell attributes. A table
total is another immutable rule over the current subtotal handles. Editing a
cell lazily updates its row and then the total without evaluating sibling rows.

When membership changes, the table consumer rebuilds the aggregate rule with the
new fixed handle set. That produces a new aggregate `NodeId`; every downstream
rule that stored the old aggregate handle must therefore be rebuilt too. Safe
removal proceeds from downstream owners toward the removed row, so no retained
rule can read a row handle after that row is removed. The core graph does not
infer collection membership or mutate installed rule definitions.

### Reused list-view cells

A viewport index is a source. The visible-cell rule reads the index and one of a
fixed set of row values. Offscreen row changes do not dirty the cell. Moving the
viewport swaps its active row dependency, modelling how a higher layer can reuse
one platform cell for different model identities.

The row-handle vector in this example is fixed inside the immutable rule.
Inserting, deleting, or reordering that handle set requires rebuilding the
visible-cell rule and any downstream rules that stored its old ID. Changing the
viewport index alone is not collection reconciliation.

### Animation frames

The mock animation driver writes fixed-point progress values and deliberately
delays the render read. Multiple ticks can arrive before that read; because the
graph is pull-based, the eventual read evaluates once using the latest progress.
Each distinct write still updates the source and marks dependency state; the
graph does not queue or coalesce frames. Equal source writes are no-ops. Progress
values that clamp to the same output still run the interpolation rule, but
unchanged-value propagation prevents redundant downstream paint work.

### Gesture state

The mock recognizer writes drag phase and live/resting offsets. While idle, live
movement is inactive. During a drag, the displayed offset switches to the live
source. On release, the adapter writes the final resting value and phase before
the renderer reads, allowing one final-state evaluation.

Deferring that read is consumer-side coalescing, not an atomic graph transaction.
A production UI layer would own any required event or frame transaction
semantics.

## Current Limitations

- There is no graph-native collection, ordering, keyed diff, or grouped removal.
- A consumer whose graph outlives its adapter must explicitly remove every node
  group before discarding the adapter.
- Conditional branches must use handles already stored in the immutable rule.
- The graph is pull-based; the UI layer decides when to render and read outputs.
- There is no multi-source transaction or animation transaction.
- Animation timing and gesture recognition remain external.
- Built-in typed values are currently `bool`, `i64`, and `String`; the examples
  use fixed-point `i64` rather than floating point geometry.
- Keep the current opaque rule-body harness on one owning thread until its
  cross-thread ownership contract is explicitly defined.

The test suite deliberately exercises the graph's low-level external-provider
surface with raw `NodeId`, `ValueStorage`, and one isolated unsafe boxed-body
helper. That helper depends on matching callback/body types, unique ownership,
valid lifetimes, exactly-once destruction, and single-threaded use. Production
UI adapters should hide it and expose typed attributes to application code.

Run only these contracts with:

```sh
cargo test --test ui_consumer_contracts
```
