//! Executable examples of how a higher UI layer can consume `AttributeGraph`.
//!
//! These are adapter contract tests, not native widget tests. The mock adapters
//! own collection membership, stable keys, rendering decisions, animation
//! ticks, gesture events, and the decision to remove nodes. The graph owns scalar
//! values, dependency discovery, invalidation, lazy evaluation, and edge/storage
//! cleanup after `remove_node` is called.

use std::any::type_name;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use attribute_graph::{
    AttributeGraph, DependencyChangeSet, Edge, EvaluationContext, GraphError, NodeId, NodeState,
    RuleDescriptor, RuleHandle, TypeDescriptor, UpdateFn, ValueStorage,
};

const I64: TypeDescriptor = TypeDescriptor::new("i64");
const STATIC_STR: TypeDescriptor = TypeDescriptor::new("&'static str");

struct SelectRule {
    selector: NodeId,
    when_false: NodeId,
    when_true: NodeId,
    updates: Rc<Cell<usize>>,
}

struct MultiplyRule {
    lhs: NodeId,
    rhs: NodeId,
    updates: Rc<Cell<usize>>,
    drops: Rc<Cell<usize>>,
}

impl Drop for MultiplyRule {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

struct SumManyRule {
    inputs: Vec<NodeId>,
    updates: Rc<Cell<usize>>,
}

struct IndexedRule {
    index: NodeId,
    values: Vec<NodeId>,
    updates: Rc<Cell<usize>>,
}

struct InterpolateRule {
    start: NodeId,
    end: NodeId,
    progress: NodeId,
    updates: Rc<Cell<usize>>,
}

struct PassThroughRule {
    input: NodeId,
    updates: Rc<Cell<usize>>,
}

fn boxed_rule<T: 'static>(
    body: T,
    update: UpdateFn,
    value_type: TypeDescriptor,
    debug_name: &'static str,
) -> RuleDescriptor {
    // This unsafe helper is isolated test infrastructure for exercising the
    // low-level external-provider ABI. It relies on unique Box ownership, a
    // matching callback/body type, a valid lifetime, exactly-once destruction,
    // and one-threaded use. A UI adapter should expose typed attributes and hide
    // this rule-body plumbing from application code.
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

fn read_i64(
    context: &mut EvaluationContext<'_>,
    dependency: NodeId,
    label: &str,
) -> Result<i64, GraphError> {
    Ok(context
        .read(dependency)?
        .as_i64()
        .unwrap_or_else(|| panic!("{label} should be an i64")))
}

fn update_select(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<SelectRule>(handle);
    rule.updates.set(rule.updates.get() + 1);

    let selector = context
        .read(rule.selector)?
        .as_bool()
        .expect("selector should be a bool");
    let selected = if selector {
        rule.when_true
    } else {
        rule.when_false
    };
    let value = context.read(selected)?;
    context.set_output(value);
    Ok(())
}

fn update_multiply(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<MultiplyRule>(handle);
    rule.updates.set(rule.updates.get() + 1);

    let lhs = read_i64(context, rule.lhs, "left table cell")?;
    let rhs = read_i64(context, rule.rhs, "right table cell")?;
    context.set_output(ValueStorage::from_i64(lhs * rhs));
    Ok(())
}

fn update_sum_many(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<SumManyRule>(handle);
    rule.updates.set(rule.updates.get() + 1);

    let mut total = 0;
    for input in &rule.inputs {
        total += read_i64(context, *input, "table row subtotal")?;
    }
    context.set_output(ValueStorage::from_i64(total));
    Ok(())
}

fn update_indexed(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<IndexedRule>(handle);
    rule.updates.set(rule.updates.get() + 1);

    let index = read_i64(context, rule.index, "visible row index")?;
    let index = usize::try_from(index).expect("the list-view adapter should provide a valid index");
    let selected = *rule
        .values
        .get(index)
        .expect("the list-view adapter should keep the index in range");
    let value = context.read(selected)?;
    context.set_output(value);
    Ok(())
}

fn update_interpolate(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<InterpolateRule>(handle);
    rule.updates.set(rule.updates.get() + 1);

    let start = read_i64(context, rule.start, "animation start")?;
    let end = read_i64(context, rule.end, "animation end")?;
    let progress = read_i64(context, rule.progress, "animation progress")?.clamp(0, 1_000);
    let value = start + ((end - start) * progress / 1_000);
    context.set_output(ValueStorage::from_i64(value));
    Ok(())
}

fn update_pass_through(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<PassThroughRule>(handle);
    rule.updates.set(rule.updates.get() + 1);
    let value = context.read(rule.input)?;
    context.set_output(value);
    Ok(())
}

#[derive(Clone, Copy)]
struct ItemModel {
    key: i64,
    price: i64,
    quantity: i64,
}

#[derive(Clone)]
struct RowNodes {
    price: NodeId,
    quantity: NodeId,
    subtotal: NodeId,
    updates: Rc<Cell<usize>>,
    drops: Rc<Cell<usize>>,
}

impl RowNodes {
    fn ids(&self) -> [NodeId; 3] {
        [self.price, self.quantity, self.subtotal]
    }
}

#[derive(Default)]
struct KeyedForEachAdapter {
    rows: BTreeMap<i64, RowNodes>,
    order: Vec<i64>,
}

#[derive(Debug, Eq, PartialEq)]
enum ReconcileError {
    DuplicateKey(i64),
    Graph(GraphError),
}

impl From<GraphError> for ReconcileError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

impl KeyedForEachAdapter {
    fn reconcile(
        &mut self,
        graph: &mut AttributeGraph,
        items: &[ItemModel],
    ) -> Result<(), ReconcileError> {
        let mut next_keys = BTreeSet::new();
        for item in items {
            if !next_keys.insert(item.key) {
                return Err(ReconcileError::DuplicateKey(item.key));
            }
        }

        let removed_keys = self
            .rows
            .keys()
            .filter(|key| !next_keys.contains(key))
            .copied()
            .collect::<Vec<_>>();

        for key in removed_keys {
            let row = self.rows.remove(&key).expect("removed key should exist");
            drop(
                graph
                    .remove_node(row.subtotal)
                    .expect("adapter-owned subtotal should exist"),
            );
            drop(
                graph
                    .remove_node(row.price)
                    .expect("adapter-owned price should exist"),
            );
            drop(
                graph
                    .remove_node(row.quantity)
                    .expect("adapter-owned quantity should exist"),
            );
        }

        for item in items {
            if let Some(row) = self.rows.get(&item.key) {
                graph.set_source_value(row.price, ValueStorage::from_i64(item.price))?;
                graph.set_source_value(row.quantity, ValueStorage::from_i64(item.quantity))?;
                continue;
            }

            let price = graph.add_source(ValueStorage::from_i64(item.price));
            let quantity = graph.add_source(ValueStorage::from_i64(item.quantity));
            let updates = Rc::new(Cell::new(0));
            let drops = Rc::new(Cell::new(0));
            let subtotal = graph.add_derived(boxed_rule(
                MultiplyRule {
                    lhs: price,
                    rhs: quantity,
                    updates: Rc::clone(&updates),
                    drops: Rc::clone(&drops),
                },
                update_multiply,
                I64,
                "table row subtotal",
            ));

            self.rows.insert(
                item.key,
                RowNodes {
                    price,
                    quantity,
                    subtotal,
                    updates,
                    drops,
                },
            );
        }

        self.order = items.iter().map(|item| item.key).collect();
        Ok(())
    }

    fn row(&self, key: i64) -> RowNodes {
        self.rows.get(&key).expect("row should exist").clone()
    }

    fn ordered_subtotals(&self, graph: &mut AttributeGraph) -> Result<Vec<(i64, i64)>, GraphError> {
        self.order
            .iter()
            .map(|key| {
                let row = self.rows.get(key).expect("ordered row should exist");
                let subtotal = graph
                    .read_value(row.subtotal)?
                    .as_i64()
                    .expect("subtotal should be an i64");
                Ok((*key, subtotal))
            })
            .collect()
    }
}

fn table_total_rule(rows: &[RowNodes], updates: Rc<Cell<usize>>) -> RuleDescriptor {
    boxed_rule(
        SumManyRule {
            inputs: rows.iter().map(|row| row.subtotal).collect(),
            updates,
        },
        update_sum_many,
        I64,
        "table grand total",
    )
}

#[test]
fn consumer_conditional_tracks_only_the_selected_content_branch() {
    let mut graph = AttributeGraph::new();
    let show_details = graph.add_source(ValueStorage::from_bool(false));
    let summary = graph.add_source(ValueStorage::from_static_str("Summary"));
    let details = graph.add_source(ValueStorage::from_static_str("Full details"));
    let updates = Rc::new(Cell::new(0));
    let visible_content = graph.add_derived(boxed_rule(
        SelectRule {
            selector: show_details,
            when_false: summary,
            when_true: details,
            updates: Rc::clone(&updates),
        },
        update_select,
        STATIC_STR,
        "visible conditional content",
    ));

    assert_eq!(
        graph.read_value(visible_content).unwrap().as_static_str(),
        Some("Summary")
    );
    assert_eq!(
        graph.dependencies_of(visible_content),
        Ok(vec![show_details, summary])
    );

    assert_eq!(
        graph.set_source_value(details, ValueStorage::from_static_str("New details")),
        Ok(vec![]),
        "inactive content should not invalidate the visible branch",
    );
    assert_eq!(
        graph.node(visible_content).unwrap().state(),
        NodeState::Clean
    );
    assert_eq!(updates.get(), 1);

    graph
        .set_source_value(show_details, ValueStorage::from_bool(true))
        .unwrap();
    assert_eq!(
        graph.read_value(visible_content).unwrap().as_static_str(),
        Some("New details")
    );
    assert_eq!(
        graph.dependencies_of(visible_content),
        Ok(vec![show_details, details])
    );

    assert_eq!(
        graph.set_source_value(summary, ValueStorage::from_static_str("New summary")),
        Ok(vec![]),
    );
    assert_eq!(
        graph.node(visible_content).unwrap().state(),
        NodeState::Clean
    );
    assert_eq!(updates.get(), 2);
}

#[test]
fn foreach_adapter_preserves_stable_rows_during_reorder_and_localizes_updates() {
    let mut graph = AttributeGraph::new();
    let mut adapter = KeyedForEachAdapter::default();
    adapter
        .reconcile(
            &mut graph,
            &[
                ItemModel {
                    key: 1,
                    price: 10,
                    quantity: 2,
                },
                ItemModel {
                    key: 2,
                    price: 5,
                    quantity: 3,
                },
                ItemModel {
                    key: 3,
                    price: 4,
                    quantity: 4,
                },
            ],
        )
        .unwrap();

    assert_eq!(
        adapter.ordered_subtotals(&mut graph).unwrap(),
        vec![(1, 20), (2, 15), (3, 16)]
    );
    let row_one = adapter.row(1);
    let row_two = adapter.row(2);
    let row_three = adapter.row(3);

    adapter
        .reconcile(
            &mut graph,
            &[
                ItemModel {
                    key: 3,
                    price: 4,
                    quantity: 4,
                },
                ItemModel {
                    key: 1,
                    price: 10,
                    quantity: 2,
                },
                ItemModel {
                    key: 2,
                    price: 5,
                    quantity: 4,
                },
            ],
        )
        .unwrap();

    assert_eq!(adapter.row(1).ids(), row_one.ids());
    assert_eq!(adapter.row(2).ids(), row_two.ids());
    assert_eq!(adapter.row(3).ids(), row_three.ids());
    assert_eq!(
        graph.node(row_one.subtotal).unwrap().state(),
        NodeState::Clean
    );
    assert_eq!(
        graph.node(row_two.subtotal).unwrap().state(),
        NodeState::Dirty
    );
    assert_eq!(
        graph.node(row_three.subtotal).unwrap().state(),
        NodeState::Clean
    );
    assert_eq!(
        graph.pending_edges(),
        vec![Edge::new(row_two.quantity, row_two.subtotal)]
    );

    assert_eq!(
        adapter.ordered_subtotals(&mut graph).unwrap(),
        vec![(3, 16), (1, 20), (2, 20)]
    );
    assert_eq!(row_one.updates.get(), 1);
    assert_eq!(row_two.updates.get(), 2);
    assert_eq!(row_three.updates.get(), 1);
    assert_eq!(graph.node_count(), 9);
    assert_eq!(graph.edge_count(), 6);
    assert!(graph.pending_edges().is_empty());
}

#[test]
fn foreach_adapter_rejects_duplicate_keys_before_graph_mutation() {
    let mut graph = AttributeGraph::new();
    let mut adapter = KeyedForEachAdapter::default();
    adapter
        .reconcile(
            &mut graph,
            &[ItemModel {
                key: 1,
                price: 10,
                quantity: 2,
            }],
        )
        .unwrap();
    assert_eq!(
        adapter.ordered_subtotals(&mut graph).unwrap(),
        vec![(1, 20)]
    );
    let original = adapter.row(1);

    assert_eq!(
        adapter.reconcile(
            &mut graph,
            &[
                ItemModel {
                    key: 1,
                    price: 99,
                    quantity: 2,
                },
                ItemModel {
                    key: 1,
                    price: 50,
                    quantity: 4,
                },
            ],
        ),
        Err(ReconcileError::DuplicateKey(1))
    );

    assert_eq!(adapter.order, vec![1]);
    assert_eq!(adapter.row(1).ids(), original.ids());
    assert_eq!(graph.node_count(), 3);
    assert_eq!(graph.edge_count(), 2);
    assert_eq!(
        graph.node(original.subtotal).unwrap().state(),
        NodeState::Clean
    );
    assert_eq!(
        graph.read_value(original.subtotal).unwrap().as_i64(),
        Some(20)
    );
    assert_eq!(original.updates.get(), 1);
    assert!(graph.pending_edges().is_empty());
}

#[test]
fn foreach_adapter_owns_deleted_row_lifetimes_and_creates_fresh_nodes() {
    let mut graph = AttributeGraph::new();
    let mut adapter = KeyedForEachAdapter::default();
    adapter
        .reconcile(
            &mut graph,
            &[
                ItemModel {
                    key: 1,
                    price: 10,
                    quantity: 2,
                },
                ItemModel {
                    key: 2,
                    price: 5,
                    quantity: 3,
                },
            ],
        )
        .unwrap();
    adapter.ordered_subtotals(&mut graph).unwrap();
    let removed_row = adapter.row(1);
    let retained_row = adapter.row(2);

    adapter
        .reconcile(
            &mut graph,
            &[
                ItemModel {
                    key: 2,
                    price: 5,
                    quantity: 3,
                },
                ItemModel {
                    key: 3,
                    price: 7,
                    quantity: 2,
                },
            ],
        )
        .unwrap();
    let new_row = adapter.row(3);

    assert_eq!(adapter.row(2).ids(), retained_row.ids());
    let removed_ids = removed_row.ids();
    for removed in removed_ids {
        assert!(!graph.contains_node(removed));
        assert!(!new_row.ids().contains(&removed));
    }
    assert_eq!(
        graph.read_value(removed_row.subtotal),
        Err(GraphError::MissingNode(removed_row.subtotal))
    );
    assert_eq!(
        adapter.ordered_subtotals(&mut graph).unwrap(),
        vec![(2, 15), (3, 14)]
    );
    assert_eq!(retained_row.updates.get(), 1);
    assert_eq!(new_row.updates.get(), 1);
    assert_eq!(removed_row.drops.get(), 1);
    assert_eq!(retained_row.drops.get(), 0);
    assert_eq!(new_row.drops.get(), 0);
    assert_eq!(graph.node_count(), 6);
    assert_eq!(graph.edge_count(), 4);
    assert!(
        graph
            .edges()
            .iter()
            .all(|edge| !removed_ids.contains(&edge.dependency)
                && !removed_ids.contains(&edge.dependent))
    );
    assert!(graph.pending_edges().is_empty());
}

#[test]
fn table_adapter_propagates_one_changed_cell_without_recomputing_sibling_rows() {
    let mut graph = AttributeGraph::new();
    let mut adapter = KeyedForEachAdapter::default();
    adapter
        .reconcile(
            &mut graph,
            &[
                ItemModel {
                    key: 1,
                    price: 10,
                    quantity: 2,
                },
                ItemModel {
                    key: 2,
                    price: 5,
                    quantity: 3,
                },
            ],
        )
        .unwrap();
    let row_one = adapter.row(1);
    let row_two = adapter.row(2);
    let total_updates = Rc::new(Cell::new(0));
    let grand_total = graph.add_derived(table_total_rule(
        &[row_one.clone(), row_two.clone()],
        Rc::clone(&total_updates),
    ));

    assert_eq!(graph.read_value(grand_total).unwrap().as_i64(), Some(35));
    adapter
        .reconcile(
            &mut graph,
            &[
                ItemModel {
                    key: 1,
                    price: 12,
                    quantity: 2,
                },
                ItemModel {
                    key: 2,
                    price: 5,
                    quantity: 3,
                },
            ],
        )
        .unwrap();

    assert_eq!(
        graph.node(row_one.subtotal).unwrap().state(),
        NodeState::Dirty
    );
    assert_eq!(
        graph.node(row_two.subtotal).unwrap().state(),
        NodeState::Clean
    );
    assert_eq!(
        graph.node(grand_total).unwrap().state(),
        NodeState::MaybeDirty
    );
    assert_eq!(row_one.updates.get(), 1);
    assert_eq!(row_two.updates.get(), 1);
    assert_eq!(total_updates.get(), 1);

    assert_eq!(graph.read_value(grand_total).unwrap().as_i64(), Some(39));
    assert_eq!(row_one.updates.get(), 2);
    assert_eq!(row_two.updates.get(), 1);
    assert_eq!(total_updates.get(), 2);
    assert!(graph.pending_edges().is_empty());
}

#[test]
fn table_adapter_rebuilds_an_aggregate_when_row_membership_changes() {
    let mut graph = AttributeGraph::new();
    let mut adapter = KeyedForEachAdapter::default();
    adapter
        .reconcile(
            &mut graph,
            &[
                ItemModel {
                    key: 1,
                    price: 10,
                    quantity: 2,
                },
                ItemModel {
                    key: 2,
                    price: 5,
                    quantity: 3,
                },
            ],
        )
        .unwrap();
    let row_one = adapter.row(1);
    let row_two = adapter.row(2);
    let first_total_updates = Rc::new(Cell::new(0));
    let first_total = graph.add_derived(table_total_rule(
        &[row_one.clone(), row_two.clone()],
        first_total_updates,
    ));
    assert_eq!(graph.read_value(first_total).unwrap().as_i64(), Some(35));

    adapter
        .reconcile(
            &mut graph,
            &[
                ItemModel {
                    key: 1,
                    price: 10,
                    quantity: 2,
                },
                ItemModel {
                    key: 2,
                    price: 5,
                    quantity: 3,
                },
                ItemModel {
                    key: 3,
                    price: 4,
                    quantity: 4,
                },
            ],
        )
        .unwrap();
    let row_three = adapter.row(3);

    drop(
        graph
            .remove_node(first_total)
            .expect("the table adapter should own its aggregate"),
    );
    let next_total_updates = Rc::new(Cell::new(0));
    let next_total = graph.add_derived(table_total_rule(
        &[row_one.clone(), row_two.clone(), row_three.clone()],
        Rc::clone(&next_total_updates),
    ));

    assert!(!graph.contains_node(first_total));
    assert_ne!(first_total, next_total);
    assert_eq!(graph.read_value(next_total).unwrap().as_i64(), Some(51));
    assert_eq!(
        graph.dependencies_of(next_total),
        Ok(vec![row_one.subtotal, row_two.subtotal, row_three.subtotal])
    );
    assert_eq!(row_one.updates.get(), 1);
    assert_eq!(row_two.updates.get(), 1);
    assert_eq!(row_three.updates.get(), 1);
    assert_eq!(next_total_updates.get(), 1);
    assert_eq!(graph.node_count(), 10);
    assert_eq!(graph.edge_count(), 9);

    // Membership-dependent owners are removed before their input row. If a
    // downstream rule stored `next_total`, the table consumer would rebuild that
    // rule first as part of the same downstream identity cascade.
    drop(
        graph
            .remove_node(next_total)
            .expect("the table consumer should remove its aggregate first"),
    );
    adapter
        .reconcile(
            &mut graph,
            &[
                ItemModel {
                    key: 2,
                    price: 5,
                    quantity: 3,
                },
                ItemModel {
                    key: 3,
                    price: 4,
                    quantity: 4,
                },
            ],
        )
        .unwrap();
    let final_total_updates = Rc::new(Cell::new(0));
    let final_total = graph.add_derived(table_total_rule(
        &[row_two.clone(), row_three.clone()],
        Rc::clone(&final_total_updates),
    ));

    for removed in row_one.ids() {
        assert!(!graph.contains_node(removed));
    }
    assert!(!graph.contains_node(next_total));
    assert_eq!(graph.read_value(final_total).unwrap().as_i64(), Some(31));
    assert_eq!(
        graph.dependencies_of(final_total),
        Ok(vec![row_two.subtotal, row_three.subtotal])
    );
    assert_eq!(adapter.row(2).ids(), row_two.ids());
    assert_eq!(adapter.row(3).ids(), row_three.ids());
    assert_eq!(row_one.drops.get(), 1);
    assert_eq!(row_two.updates.get(), 1);
    assert_eq!(row_three.updates.get(), 1);
    assert_eq!(final_total_updates.get(), 1);
    assert_eq!(graph.node_count(), 7);
    assert_eq!(graph.edge_count(), 6);
    assert!(graph.pending_edges().is_empty());
}

#[test]
fn list_view_adapter_tracks_only_the_row_used_by_a_reused_visible_cell() {
    let mut graph = AttributeGraph::new();
    let visible_index = graph.add_source(ValueStorage::from_i64(0));
    let first = graph.add_source(ValueStorage::from_static_str("First"));
    let second = graph.add_source(ValueStorage::from_static_str("Second"));
    let third = graph.add_source(ValueStorage::from_static_str("Third"));
    let updates = Rc::new(Cell::new(0));
    let visible_cell = graph.add_derived(boxed_rule(
        IndexedRule {
            index: visible_index,
            values: vec![first, second, third],
            updates: Rc::clone(&updates),
        },
        update_indexed,
        STATIC_STR,
        "reused visible list cell",
    ));

    assert_eq!(
        graph.read_value(visible_cell).unwrap().as_static_str(),
        Some("First")
    );
    assert_eq!(
        graph.dependencies_of(visible_cell),
        Ok(vec![visible_index, first])
    );
    assert_eq!(
        graph.set_source_value(third, ValueStorage::from_static_str("Updated third")),
        Ok(vec![]),
        "an offscreen row should not invalidate the reused cell",
    );

    graph
        .set_source_value(visible_index, ValueStorage::from_i64(2))
        .unwrap();
    assert_eq!(
        graph.read_value(visible_cell).unwrap().as_static_str(),
        Some("Updated third")
    );
    assert_eq!(
        graph.dependencies_of(visible_cell),
        Ok(vec![visible_index, third])
    );
    assert_eq!(
        graph.set_source_value(first, ValueStorage::from_static_str("Updated first")),
        Ok(vec![]),
    );
    assert_eq!(graph.node(visible_cell).unwrap().state(), NodeState::Clean);
    assert_eq!(updates.get(), 2);
}

#[test]
fn animation_driver_defers_the_render_read_and_skips_equal_downstream_work() {
    let mut graph = AttributeGraph::new();
    let start = graph.add_source(ValueStorage::from_i64(0));
    let end = graph.add_source(ValueStorage::from_i64(100));
    let progress = graph.add_source(ValueStorage::from_i64(0));
    let interpolation_updates = Rc::new(Cell::new(0));
    let position = graph.add_derived(boxed_rule(
        InterpolateRule {
            start,
            end,
            progress,
            updates: Rc::clone(&interpolation_updates),
        },
        update_interpolate,
        I64,
        "animation interpolation",
    ));
    let paint_updates = Rc::new(Cell::new(0));
    let paint_command = graph.add_derived(boxed_rule(
        PassThroughRule {
            input: position,
            updates: Rc::clone(&paint_updates),
        },
        update_pass_through,
        I64,
        "mock paint command",
    ));

    assert_eq!(graph.read_value(paint_command).unwrap().as_i64(), Some(0));
    graph
        .set_source_value(progress, ValueStorage::from_i64(250))
        .unwrap();
    graph
        .set_source_value(progress, ValueStorage::from_i64(500))
        .unwrap();
    assert_eq!(interpolation_updates.get(), 1);
    assert_eq!(paint_updates.get(), 1);
    assert_eq!(graph.node(position).unwrap().state(), NodeState::Dirty);
    assert_eq!(
        graph.node(paint_command).unwrap().state(),
        NodeState::MaybeDirty
    );

    assert_eq!(graph.read_value(paint_command).unwrap().as_i64(), Some(50));
    assert_eq!(interpolation_updates.get(), 2);
    assert_eq!(paint_updates.get(), 2);
    assert_eq!(
        graph.set_source_value(progress, ValueStorage::from_i64(500)),
        Ok(vec![]),
        "an equal animation tick should be a no-op",
    );

    graph
        .set_source_value(progress, ValueStorage::from_i64(1_500))
        .unwrap();
    assert_eq!(graph.read_value(paint_command).unwrap().as_i64(), Some(100));
    assert_eq!(interpolation_updates.get(), 3);
    assert_eq!(paint_updates.get(), 3);

    graph
        .set_source_value(progress, ValueStorage::from_i64(1_600))
        .unwrap();
    assert_eq!(graph.read_value(paint_command).unwrap().as_i64(), Some(100));
    assert_eq!(interpolation_updates.get(), 4);
    assert_eq!(
        paint_updates.get(),
        3,
        "an unchanged interpolated value should clean downstream work lazily",
    );
}

#[test]
fn gesture_adapter_writes_live_and_resting_sources_before_rendering() {
    let mut graph = AttributeGraph::new();
    let is_dragging = graph.add_source(ValueStorage::from_bool(false));
    let resting_offset = graph.add_source(ValueStorage::from_i64(0));
    let live_offset = graph.add_source(ValueStorage::from_i64(0));
    let updates = Rc::new(Cell::new(0));
    let displayed_offset = graph.add_derived(boxed_rule(
        SelectRule {
            selector: is_dragging,
            when_false: resting_offset,
            when_true: live_offset,
            updates: Rc::clone(&updates),
        },
        update_select,
        I64,
        "drag gesture display offset",
    ));

    assert_eq!(
        graph.read_value(displayed_offset).unwrap().as_i64(),
        Some(0)
    );
    assert_eq!(
        graph.dependencies_of(displayed_offset),
        Ok(vec![is_dragging, resting_offset])
    );
    assert_eq!(
        graph.set_source_value(live_offset, ValueStorage::from_i64(12)),
        Ok(vec![]),
        "recognizer movement is irrelevant before the drag begins",
    );

    graph
        .set_source_value(is_dragging, ValueStorage::from_bool(true))
        .unwrap();
    assert_eq!(
        graph.read_value(displayed_offset).unwrap().as_i64(),
        Some(12)
    );
    graph
        .set_source_value(live_offset, ValueStorage::from_i64(24))
        .unwrap();
    assert_eq!(
        graph.read_value(displayed_offset).unwrap().as_i64(),
        Some(24)
    );
    assert_eq!(
        graph.dependencies_of(displayed_offset),
        Ok(vec![is_dragging, live_offset])
    );

    // The gesture adapter writes the final resting value and phase before the
    // renderer reads. This coalesces final-state evaluation, but it is not a
    // graph-level multi-source transaction.
    graph
        .set_source_value(resting_offset, ValueStorage::from_i64(24))
        .unwrap();
    graph
        .set_source_value(is_dragging, ValueStorage::from_bool(false))
        .unwrap();
    let release = graph.update_node(displayed_offset).unwrap();
    assert!(!release.value_changed);
    assert_eq!(
        release.dependency_changes,
        DependencyChangeSet {
            added: vec![resting_offset],
            removed: vec![live_offset],
            retained: vec![is_dragging],
        }
    );
    assert_eq!(
        graph.dependencies_of(displayed_offset),
        Ok(vec![is_dragging, resting_offset])
    );
    assert_eq!(
        graph.read_value(displayed_offset).unwrap().as_i64(),
        Some(24)
    );

    assert_eq!(
        graph.set_source_value(live_offset, ValueStorage::from_i64(30)),
        Ok(vec![]),
        "movement after release should not invalidate resting content",
    );
    assert_eq!(
        graph.node(displayed_offset).unwrap().state(),
        NodeState::Clean
    );
    assert_eq!(updates.get(), 4);
}
