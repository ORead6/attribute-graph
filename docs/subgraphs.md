# Subgraph V1 Contract

A subgraph is an ownership and lifetime region inside one `AttributeGraph`. It
groups nodes that should be torn down together. It is not a nested graph, an
evaluation boundary, or a scheduling unit: nodes in different subgraphs of the
same graph may depend on one another normally.

## Construction ownership

A graph may enter a subgraph construction scope. Every node created through the
graph while that scope is active is owned by the innermost active scope,
including nodes created indirectly by helper code. Nested construction scopes
restore the enclosing scope when they finish.

A node has at most one owning subgraph. Nodes created without an active scope
remain unowned and continue to use the ordinary individual-node lifetime. A
construction scope assigns ownership; leaving the scope does not remove its
nodes or roll construction back.

`build_subgraph` is the transactional convenience for creating a new region. If
its builder returns a `GraphError` or panics, that newly created subgraph and its
descendants are removed before the error or panic continues. This rollback is
limited to the new ownership subtree: writes to pre-existing nodes and other
explicit mutations are not graph-wide transactional state.

Panic cleanup requires Rust's unwinding panic mode. With `panic=abort`, the
process terminates and neither scope restoration nor rollback can run.

## Nesting

Subgraphs may have a parent in the same graph. Parentage is fixed when the child
is created and cannot be changed in v1. This makes the ownership hierarchy a
forest and gives it one unambiguous lifetime rule: a child cannot outlive its
parent.

Nesting affects ownership only. It does not alter dependency discovery, cache
validation, or evaluation order.

## Permanent teardown

Removing a subgraph permanently removes that subgraph, all descendant
subgraphs, and every node they own. The operation is atomic at the public graph
boundary: callers cannot evaluate or inspect the graph between the individual
member removals.

Before the operation returns, the graph:

1. identifies the complete ownership subtree;
2. invalidates surviving dependents connected to removed nodes by committed,
   active dependency edges;
3. detaches active and pending edges involving the removed nodes;
4. removes the owned nodes and subgraph records; and
5. invokes rule destroy callbacks only after structural detachment is complete.

Destroy callbacks are release-only hooks for provider-owned opaque storage.
They must not read, evaluate, or mutate the graph, re-enter graph operations, or
perform graph-dependent cleanup. They should not panic. All callbacks run after
structural detachment and exactly once, but their relative order is not a public
contract and must not be used to coordinate ownership between rule bodies.

Removal is permanent. V1 has no detach, suspension, reinsertion, or
identity-preserving reuse operation.

## Surviving and stale handles

Node and subgraph IDs are scoped to their owning graph, allocated
monotonically, and never reused within that graph. Removing a region therefore
cannot cause an old handle to refer to a newly created object. Reading a removed
node handle produces `MissingNode`.

The graph can invalidate only dependencies that were discovered by successful
rule reads and are present as committed active edges. A rule may contain other
opaque node handles that its current conditional path has not read. Those
inactive stored handles cannot be discovered or rewritten by the graph. If a
later evaluation chooses one after its target has been removed, that evaluation
fails with `MissingNode`; the owning layer must avoid that path or rebuild the
rule with live handles.

Handles and dependencies may cross subgraph boundaries within the same
`AttributeGraph`. They may not cross graph boundaries.

## Deliberate non-goals

Subgraphs do not provide:

- detach/reinsert, suspension, or identity-preserving reuse;
- a scheduler, push notifications, render passes, or rendering;
- conditional-graph or control-flow semantics;
- a nested evaluation graph or cross-graph dependencies;
- collection diffing, ordering, keyed identity, or UI component behavior.

Those remain responsibilities of a layer built above the fundamental attribute
graph.
