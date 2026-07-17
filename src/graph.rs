use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use crate::attribute::{Attribute, DynamicAttribute, StaticAttribute, decode_attribute_value};
use crate::dependency::{DependencyChangeSet, Edge, EdgeState, UpdateOutcome, sorted_ids};
use crate::error::GraphError;
use crate::identity::{GraphId, NodeId, next_graph_id};
use crate::node::{Node, NodeKind, NodeState};
use crate::rule::RuleDescriptor;
use crate::value::{AttributeValue, TypeDescriptor, ValueStorage};

/// Runtime storage for attributes, dependencies, dirty state, and rule dispatch.
///
/// Notice what is not here: no concrete rule types and no Swift/Rust UI logic.
/// The graph just knows how to call an update function, provide an evaluation
/// context, observe reads, and cache the resulting value.
#[derive(Debug)]
pub struct AttributeGraph {
    // Hash maps keep the first pass easy to read. An arena can replace this
    // later once the behavior is right and stable node indices matter.
    id: GraphId,
    nodes: HashMap<NodeId, Node>,
    dependents: HashMap<NodeId, HashSet<NodeId>>,
    pending_edges: HashSet<Edge>,
    evaluation_stack: Vec<NodeId>,
    next_node_id: u64,
}

impl Default for AttributeGraph {
    fn default() -> Self {
        Self {
            id: next_graph_id(),
            nodes: HashMap::new(),
            dependents: HashMap::new(),
            pending_edges: HashSet::new(),
            evaluation_stack: Vec::new(),
            next_node_id: 0,
        }
    }
}

impl AttributeGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return this graph's stable runtime identity.
    pub const fn id(&self) -> GraphId {
        self.id
    }

    pub fn add_static_attribute<T: AttributeValue>(&mut self, value: T) -> StaticAttribute<T> {
        StaticAttribute::new(self.add_source(value.into_storage()))
    }

    pub fn add_dynamic_attribute<T: AttributeValue>(
        &mut self,
        rule: RuleDescriptor,
    ) -> Result<DynamicAttribute<T>, GraphError> {
        let expected = T::type_descriptor();
        let actual = rule.value_type();

        if expected != actual {
            return Err(GraphError::RuleValueTypeMismatch { expected, actual });
        }

        Ok(DynamicAttribute::new(self.add_derived(rule)))
    }

    pub fn read<T, A>(&mut self, attribute: A) -> Result<T, GraphError>
    where
        T: AttributeValue,
        A: Into<Attribute<T>>,
    {
        let attribute = attribute.into();
        let value = self.read_value(attribute.id())?;
        decode_attribute_value(attribute.id(), &value)
    }

    pub fn set_static<T: AttributeValue>(
        &mut self,
        attribute: StaticAttribute<T>,
        value: T,
    ) -> Result<(), GraphError> {
        self.set_source_value(attribute.id(), value.into_storage())
            .map(|_| ())
    }

    pub fn update_dynamic<T: AttributeValue>(
        &mut self,
        attribute: DynamicAttribute<T>,
    ) -> Result<UpdateOutcome, GraphError> {
        self.update_node(attribute.id())
    }

    /// Add an externally supplied value.
    ///
    /// Source nodes are immediately valid because their value comes from outside
    /// the graph. When a source changes, dependents are marked dirty.
    pub fn add_source(&mut self, value: ValueStorage) -> NodeId {
        let id = self.next_id();

        self.nodes.insert(
            id,
            Node {
                id,
                kind: NodeKind::Source,
                state: NodeState::Clean,
                value: Some(value),
                active_dependencies: HashSet::new(),
                rule: None,
            },
        );
        self.dependents.entry(id).or_default();

        id
    }

    /// Add a derived value calculated by an externally supplied rule.
    ///
    /// The graph stores the opaque rule body and update function, but does not
    /// know the rule's concrete type. The node starts dirty with no cached value;
    /// the first `update_node` call runs the rule and fills the cache.
    pub fn add_derived(&mut self, rule: RuleDescriptor) -> NodeId {
        let id = self.next_id();

        self.nodes.insert(
            id,
            Node {
                id,
                kind: NodeKind::Derived,
                state: NodeState::Dirty,
                value: None,
                active_dependencies: HashSet::new(),
                rule: Some(rule),
            },
        );
        self.dependents.entry(id).or_default();

        id
    }

    /// Remove a node and invalidate every dependent that may have cached its value.
    ///
    /// Direct dependents become dirty and further descendants become maybe-dirty
    /// before the removed node's edges are detached. A later read therefore runs
    /// the affected rule again instead of returning a stale cached value. If that
    /// rule still reads the removed node, the read returns [`GraphError::MissingNode`].
    pub fn remove_node(&mut self, id: NodeId) -> Option<Node> {
        if !self.nodes.contains_key(&id) {
            return None;
        }

        self.mark_changed(id)
            .expect("a node checked above should be valid in this graph");
        let node = self.nodes.remove(&id)?;

        for dependency in &node.active_dependencies {
            if let Some(dependents) = self.dependents.get_mut(dependency) {
                dependents.remove(&id);
            }
        }

        if let Some(dependents) = self.dependents.remove(&id) {
            for dependent in dependents {
                if let Some(dependent_node) = self.nodes.get_mut(&dependent) {
                    dependent_node.active_dependencies.remove(&id);
                }
            }
        }

        self.pending_edges
            .retain(|edge| edge.dependency != id && edge.dependent != id);

        Some(node)
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    #[doc(hidden)]
    pub fn debug_cached_value(&self, id: NodeId) -> Option<&ValueStorage> {
        self.nodes.get(&id)?.debug_cached_value()
    }

    pub fn contains_node(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.dependents.values().map(HashSet::len).sum()
    }

    /// Write a new value into a source node and mark dependents stale if it changed.
    ///
    /// This is the "external state changed" entry point. It does not eagerly
    /// recompute anything; it just records pending work. An equal bytewise value
    /// is a no-op and returns an empty dependent list. Values using
    /// [`crate::ValueComparison::AlwaysChanged`] always invalidate.
    pub fn set_source_value(
        &mut self,
        id: NodeId,
        value: ValueStorage,
    ) -> Result<Vec<NodeId>, GraphError> {
        self.ensure_source(id)?;
        self.ensure_node_value_type(id, value.value_type())?;

        let node = self.nodes.get_mut(&id).expect("source node should exist");
        let value_changed = value.meaningfully_changed_from(node.value.as_ref());
        node.value = Some(value);
        node.state = NodeState::Clean;

        if value_changed {
            self.mark_changed(id)
        } else {
            Ok(Vec::new())
        }
    }

    /// Read a node through the lazy validation path.
    ///
    /// Callers should prefer this over inspecting `Node::value()` directly: it
    /// validates dirty and maybe-dirty ancestors before returning the cached value.
    pub fn read_value(&mut self, id: NodeId) -> Result<ValueStorage, GraphError> {
        self.validate_node_for_read(id)?;
        self.cached_value(id)
    }

    /// Recompute or validate a derived node if its cached value may be stale.
    ///
    /// This is the core dispatch loop:
    /// 1. Find the derived node's opaque rule body and update function.
    /// 2. Build an `EvaluationContext`.
    /// 3. Let the external callback run.
    /// 4. Replace the node's dependencies with the nodes the callback read.
    /// 5. Compare the old cached value with the new output.
    /// 6. Store the callback's output as the cached value.
    /// 7. If the value changed, dirty downstream dependents automatically.
    ///
    /// If the callback returns an error, fails to set output, reads a missing
    /// dependency, or produces an invalid output, this node remains dirty. Its
    /// prior cache and dependency set remain intact, pending input edges are not
    /// consumed, and a later call retries evaluation. Successful nested updates
    /// performed while reading dependencies are retained; evaluation is atomic
    /// for this node, not a graph-wide transaction. An unwinding callback panic
    /// is resumed after the graph restores its evaluation stack, so callers that
    /// catch the unwind can retry or otherwise continue using the graph.
    pub fn update_node(&mut self, id: NodeId) -> Result<UpdateOutcome, GraphError> {
        self.ensure_derived(id)?;
        self.validate_derived_node(id)
    }

    /// Mark dependents of `dependency` stale.
    ///
    /// Direct dependents are `Dirty` because one of their inputs definitely changed.
    /// Further descendants become `MaybeDirty`: their cache is still usable if the
    /// dirty node eventually recomputes to the same value. This keeps propagation
    /// lazy without letting distant cached values pretend they are definitely clean.
    pub fn mark_changed(&mut self, dependency: NodeId) -> Result<Vec<NodeId>, GraphError> {
        self.ensure_node(dependency)?;

        let dependents = sorted_ids(self.dependents.get(&dependency));
        for dependent in &dependents {
            if let Some(node) = self.nodes.get_mut(dependent) {
                node.state = NodeState::Dirty;
            }
            self.pending_edges.insert(Edge::new(dependency, *dependent));
        }

        let mut queue = VecDeque::from(dependents.clone());
        let mut seen = dependents.iter().copied().collect::<HashSet<_>>();

        while let Some(maybe_changed_dependency) = queue.pop_front() {
            for dependent in sorted_ids(self.dependents.get(&maybe_changed_dependency)) {
                self.pending_edges
                    .insert(Edge::new(maybe_changed_dependency, dependent));

                if seen.insert(dependent) {
                    if let Some(node) = self.nodes.get_mut(&dependent)
                        && node.state == NodeState::Clean
                    {
                        node.state = NodeState::MaybeDirty;
                    }
                    queue.push_back(dependent);
                }
            }
        }

        Ok(dependents)
    }

    fn validate_derived_node(&mut self, id: NodeId) -> Result<UpdateOutcome, GraphError> {
        self.ensure_derived(id)?;

        let (state, has_value) = {
            let node = self.nodes.get(&id).expect("derived node should exist");
            (node.state, node.value.is_some())
        };

        if state == NodeState::Clean && has_value {
            return Ok(UpdateOutcome::default());
        }

        if state == NodeState::Dirty || !has_value {
            return self.recompute_derived_node(id);
        }

        let dependencies = self
            .nodes
            .get(&id)
            .expect("derived node should exist")
            .active_dependencies();
        let mut dependency_changed = false;

        for dependency in dependencies {
            if self.validate_node_for_read(dependency)? {
                dependency_changed = true;
            }
        }

        if dependency_changed {
            self.recompute_derived_node(id)
        } else {
            self.mark_clean(id);
            self.mark_unchanged(id);
            Ok(UpdateOutcome::default())
        }
    }

    fn recompute_derived_node(&mut self, id: NodeId) -> Result<UpdateOutcome, GraphError> {
        self.ensure_derived(id)?;

        let old_value = {
            let node = self.nodes.get(&id).expect("derived node should exist");
            node.value.clone()
        };

        if self.evaluation_stack.contains(&id) {
            return Err(GraphError::CycleDetected);
        }

        let (body, update, expected_value_type) = {
            let rule = self
                .nodes
                .get(&id)
                .expect("derived node should exist")
                .rule
                .as_ref()
                .expect("derived node should have a rule");
            (rule.body(), rule.update(), rule.value_type())
        };

        self.evaluation_stack.push(id);

        let (update_result, dependencies_read, output) = {
            let mut context = EvaluationContext {
                graph: self,
                evaluating: id,
                dependencies_read: HashSet::new(),
                output: None,
            };
            let update_result = catch_unwind(AssertUnwindSafe(|| update(body, &mut context)));
            (update_result, context.dependencies_read, context.output)
        };

        let popped = self.evaluation_stack.pop();
        debug_assert_eq!(popped, Some(id));

        let update_result = match update_result {
            Ok(result) => result,
            Err(payload) => resume_unwind(payload),
        };
        update_result?;

        let output = output.ok_or(GraphError::MissingOutput(id))?;
        if output.value_type() != expected_value_type {
            return Err(GraphError::ValueTypeMismatch {
                node: id,
                expected: expected_value_type,
                actual: output.value_type(),
            });
        }

        let changes = self.commit_dependencies(id, dependencies_read)?;
        let value_changed = output.meaningfully_changed_from(old_value.as_ref());
        let node = self.nodes.get_mut(&id).expect("derived node should exist");
        node.value = Some(output);
        node.state = NodeState::Clean;
        self.clear_inbound_pending_edges(id);

        let dirtied_dependents = if value_changed {
            self.mark_changed(id)?
        } else {
            self.mark_unchanged(id);
            Vec::new()
        };

        Ok(UpdateOutcome {
            dependency_changes: changes,
            value_changed,
            dirtied_dependents,
        })
    }

    /// Commit the dependency set observed during a successful rule evaluation.
    ///
    /// This stays internal so callers cannot detach a clean node from its actual
    /// inputs. Evaluation commits the attributes read by an immutable rule;
    /// removal only detaches edges involving the removed node.
    fn commit_dependencies<I>(
        &mut self,
        dependent: NodeId,
        dependencies: I,
    ) -> Result<DependencyChangeSet, GraphError>
    where
        I: IntoIterator<Item = NodeId>,
    {
        self.ensure_derived(dependent)?;

        let next_dependencies = dependencies.into_iter().collect::<HashSet<_>>();
        for dependency in &next_dependencies {
            self.ensure_node(*dependency)?;

            if *dependency == dependent {
                return Err(GraphError::SelfDependency(dependent));
            }

            if self.has_path(dependent, *dependency) {
                return Err(GraphError::CycleDetected);
            }
        }

        let current_dependencies = self
            .nodes
            .get(&dependent)
            .expect("dependent node should exist")
            .active_dependencies
            .clone();

        let mut added = next_dependencies
            .difference(&current_dependencies)
            .copied()
            .collect::<Vec<_>>();
        let mut removed = current_dependencies
            .difference(&next_dependencies)
            .copied()
            .collect::<Vec<_>>();
        let mut retained = current_dependencies
            .intersection(&next_dependencies)
            .copied()
            .collect::<Vec<_>>();

        added.sort();
        removed.sort();
        retained.sort();

        for dependency in &removed {
            if let Some(dependents) = self.dependents.get_mut(dependency) {
                dependents.remove(&dependent);
            }
            self.pending_edges
                .remove(&Edge::new(*dependency, dependent));
        }

        for dependency in &added {
            self.dependents
                .entry(*dependency)
                .or_default()
                .insert(dependent);
        }

        self.nodes
            .get_mut(&dependent)
            .expect("dependent node should exist")
            .active_dependencies = next_dependencies;

        Ok(DependencyChangeSet {
            added,
            removed,
            retained,
        })
    }

    pub fn dependencies_of(&self, id: NodeId) -> Result<Vec<NodeId>, GraphError> {
        self.ensure_node(id)?;
        Ok(self
            .nodes
            .get(&id)
            .expect("node should exist")
            .active_dependencies())
    }

    pub fn dependents_of(&self, id: NodeId) -> Result<Vec<NodeId>, GraphError> {
        self.ensure_node(id)?;
        Ok(sorted_ids(self.dependents.get(&id)))
    }

    pub fn edges(&self) -> Vec<Edge> {
        let mut edges = self
            .dependents
            .iter()
            .flat_map(|(dependency, dependents)| {
                dependents.iter().map(|dependent| Edge {
                    dependency: *dependency,
                    dependent: *dependent,
                })
            })
            .collect::<Vec<_>>();
        edges.sort();
        edges
    }

    pub fn pending_edges(&self) -> Vec<Edge> {
        let mut edges = self.pending_edges.iter().copied().collect::<Vec<_>>();
        edges.sort();
        edges
    }

    pub fn edge_state(
        &self,
        dependency: NodeId,
        dependent: NodeId,
    ) -> Result<EdgeState, GraphError> {
        self.ensure_node(dependency)?;
        self.ensure_node(dependent)?;

        let is_active = self
            .nodes
            .get(&dependent)
            .expect("dependent node should exist")
            .active_dependencies
            .contains(&dependency);

        if !is_active {
            return Ok(EdgeState::Inactive);
        }

        if self
            .pending_edges
            .contains(&Edge::new(dependency, dependent))
        {
            Ok(EdgeState::Pending)
        } else {
            Ok(EdgeState::Settled)
        }
    }

    pub fn topological_order(&self) -> Result<Vec<NodeId>, GraphError> {
        let mut in_degree = self
            .nodes
            .iter()
            .map(|(id, node)| (*id, node.active_dependencies.len()))
            .collect::<HashMap<_, _>>();

        let mut ready = in_degree
            .iter()
            .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
            .collect::<Vec<_>>();
        ready.sort();

        let mut ready = VecDeque::from(ready);
        let mut order = Vec::with_capacity(self.nodes.len());

        while let Some(id) = ready.pop_front() {
            order.push(id);

            for dependent in sorted_ids(self.dependents.get(&id)) {
                let degree = in_degree
                    .get_mut(&dependent)
                    .expect("dependent node should be tracked");
                *degree -= 1;

                if *degree == 0 {
                    insert_sorted(&mut ready, dependent);
                }
            }
        }

        if order.len() == self.nodes.len() {
            Ok(order)
        } else {
            Err(GraphError::CycleDetected)
        }
    }

    fn validate_node_for_read(&mut self, id: NodeId) -> Result<bool, GraphError> {
        self.ensure_node(id)?;

        let kind = {
            let node = self.nodes.get(&id).expect("node should exist");
            node.kind
        };

        match kind {
            NodeKind::Source => Ok(false),
            NodeKind::Derived => Ok(self.validate_derived_node(id)?.value_changed),
        }
    }

    fn mark_clean(&mut self, id: NodeId) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.state = NodeState::Clean;
        }
        self.clear_inbound_pending_edges(id);
    }

    fn mark_unchanged(&mut self, dependency: NodeId) {
        let mut queue = VecDeque::from([dependency]);
        let mut seen = HashSet::new();

        while let Some(current) = queue.pop_front() {
            if !seen.insert(current) {
                continue;
            }

            for dependent in sorted_ids(self.dependents.get(&current)) {
                let removed_pending = self.pending_edges.remove(&Edge::new(current, dependent));

                if self.has_inbound_pending(dependent) {
                    continue;
                }

                if let Some(node) = self.nodes.get_mut(&dependent) {
                    match node.state {
                        NodeState::MaybeDirty => {
                            node.state = NodeState::Clean;
                            queue.push_back(dependent);
                        }
                        NodeState::Clean if removed_pending => {
                            queue.push_back(dependent);
                        }
                        NodeState::Clean => {}
                        NodeState::Dirty => {}
                    }
                }
            }
        }
    }

    fn clear_inbound_pending_edges(&mut self, id: NodeId) {
        self.pending_edges.retain(|edge| edge.dependent != id);
    }

    fn has_inbound_pending(&self, id: NodeId) -> bool {
        self.pending_edges.iter().any(|edge| edge.dependent == id)
    }

    fn cached_value(&self, id: NodeId) -> Result<ValueStorage, GraphError> {
        self.nodes
            .get(&id)
            .ok_or(GraphError::MissingNode(id))?
            .value
            .clone()
            .ok_or(GraphError::MissingValue(id))
    }

    fn ensure_node(&self, id: NodeId) -> Result<(), GraphError> {
        if id.graph_id() != self.id {
            return Err(GraphError::GraphMismatch {
                expected: self.id,
                actual: id.graph_id(),
            });
        }

        if self.nodes.contains_key(&id) {
            Ok(())
        } else {
            Err(GraphError::MissingNode(id))
        }
    }

    fn ensure_source(&self, id: NodeId) -> Result<(), GraphError> {
        self.ensure_node(id)?;

        if self.nodes.get(&id).expect("node should exist").kind == NodeKind::Source {
            Ok(())
        } else {
            Err(GraphError::NotSource(id))
        }
    }

    fn ensure_derived(&self, id: NodeId) -> Result<(), GraphError> {
        self.ensure_node(id)?;

        if self.nodes.get(&id).expect("node should exist").kind == NodeKind::Derived {
            Ok(())
        } else {
            Err(GraphError::NotDerived(id))
        }
    }

    fn ensure_node_value_type(&self, id: NodeId, actual: TypeDescriptor) -> Result<(), GraphError> {
        let expected = self
            .nodes
            .get(&id)
            .and_then(Node::value_type)
            .ok_or(GraphError::MissingNode(id))?;

        if expected == actual {
            Ok(())
        } else {
            Err(GraphError::ValueTypeMismatch {
                node: id,
                expected,
                actual,
            })
        }
    }

    fn has_path(&self, from: NodeId, to: NodeId) -> bool {
        let mut seen = HashSet::new();
        let mut stack = vec![from];

        while let Some(current) = stack.pop() {
            if current == to {
                return true;
            }

            if seen.insert(current) {
                stack.extend(self.dependents.get(&current).into_iter().flatten());
            }
        }

        false
    }

    fn next_id(&mut self) -> NodeId {
        let id = NodeId::new(self.id, self.next_node_id);
        self.next_node_id = self
            .next_node_id
            .checked_add(1)
            .expect("attribute graph exhausted its node id space");
        id
    }
}

/// The only API an external rule needs while it is executing.
///
/// The context is how the graph observes dependency reads. A rule should not
/// reach into graph internals. Instead it reads inputs through `read`, computes
/// a value using its own external logic, then writes the result with
/// `set_output`.
pub struct EvaluationContext<'graph> {
    graph: &'graph mut AttributeGraph,
    evaluating: NodeId,
    dependencies_read: HashSet<NodeId>,
    output: Option<ValueStorage>,
}

impl EvaluationContext<'_> {
    pub const fn evaluating(&self) -> NodeId {
        self.evaluating
    }

    /// Read another node and record it as a dependency of the current node.
    ///
    /// If the dependency is derived and dirty, the graph updates it before
    /// returning its cached value. That gives nested derived rules the same lazy
    /// spreadsheet-like behavior as AttributeGraph.
    pub fn read(&mut self, dependency: NodeId) -> Result<ValueStorage, GraphError> {
        if dependency == self.evaluating {
            return Err(GraphError::SelfDependency(self.evaluating));
        }

        self.graph.validate_node_for_read(dependency)?;
        self.dependencies_read.insert(dependency);
        self.graph.cached_value(dependency)
    }

    pub fn read_attribute<T, A>(&mut self, dependency: A) -> Result<T, GraphError>
    where
        T: AttributeValue,
        A: Into<Attribute<T>>,
    {
        let dependency = dependency.into();
        let value = self.read(dependency.id())?;
        decode_attribute_value(dependency.id(), &value)
    }

    /// Store the derived node's newly computed output.
    ///
    /// The graph validates this value's type against the node's declared output
    /// type after the update callback returns.
    pub fn set_output(&mut self, value: ValueStorage) {
        self.output = Some(value);
    }

    pub fn set_output_value<T: AttributeValue>(&mut self, value: T) {
        self.set_output(value.into_storage());
    }
}

fn insert_sorted(queue: &mut VecDeque<NodeId>, id: NodeId) {
    let index = queue
        .iter()
        .position(|queued| id < *queued)
        .unwrap_or(queue.len());
    queue.insert(index, id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::RuleHandle;

    const TEST_I64: TypeDescriptor = TypeDescriptor::new("i64");

    fn update_test_constant(
        _body: RuleHandle,
        context: &mut EvaluationContext<'_>,
    ) -> Result<(), GraphError> {
        context.set_output(ValueStorage::from_i64(1));
        Ok(())
    }

    fn test_rule(name: &'static str) -> RuleDescriptor {
        RuleDescriptor::new(
            RuleHandle::from_raw(0),
            update_test_constant,
            TypeDescriptor::new("test constant"),
            TEST_I64,
            name,
        )
    }

    #[test]
    fn dependency_commit_rejects_a_cycle_without_partial_mutation() {
        let mut graph = AttributeGraph::new();
        let a = graph.add_derived(test_rule("a"));
        let b = graph.add_derived(test_rule("b"));
        let c = graph.add_derived(test_rule("c"));

        graph.commit_dependencies(b, [a]).unwrap();
        graph.commit_dependencies(c, [b]).unwrap();

        assert_eq!(
            graph.commit_dependencies(a, [c]),
            Err(GraphError::CycleDetected)
        );
        assert_eq!(graph.edges(), vec![Edge::new(a, b), Edge::new(b, c)]);
        assert_eq!(graph.topological_order(), Ok(vec![a, b, c]));
    }
}
