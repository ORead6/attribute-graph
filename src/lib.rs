use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_GRAPH_ID: AtomicU64 = AtomicU64::new(1);

/// Stable identity for one [`AttributeGraph`] instance.
///
/// Graph ids make it possible to reject a node handle from another graph even
/// when both graphs have assigned the same graph-local node number.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GraphId(u64);

impl GraphId {
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for GraphId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "g{}", self.raw())
    }
}

/// Stable handle for a node inside the graph.
///
/// The graph owns the actual node storage. `NodeId` is intentionally tiny and
/// copyable so external layers can hold onto handles without borrowing the
/// graph. A Swift bridge, for example, could store this beside an Attribute
/// value and pass it back when it wants to read or update that attribute.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId {
    graph: GraphId,
    local: u64,
}

impl NodeId {
    const fn new(graph: GraphId, local: u64) -> Self {
        Self { graph, local }
    }

    /// Return the graph that owns this node.
    pub const fn graph_id(self) -> GraphId {
        self.graph
    }

    /// Return the graph-local node number.
    ///
    /// This number is useful for compact labels, but it is not globally unique.
    /// Use the complete `NodeId` when storing or comparing node identities.
    pub const fn raw(self) -> u64 {
        self.local
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:n{}", self.graph_id(), self.raw())
    }
}

/// A typed handle to a graph node that stores or produces `T`.
///
/// This is the public identity layer callers should pass around instead of raw
/// `NodeId`s. Static and dynamic attributes both erase to this common handle so
/// rules can depend on either kind without caring how the value is produced.
#[derive(Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Attribute<T> {
    id: NodeId,
    _value: PhantomData<fn() -> T>,
}

impl<T> Clone for Attribute<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Attribute<T> {}

impl<T> Attribute<T> {
    const fn new(id: NodeId) -> Self {
        Self {
            id,
            _value: PhantomData,
        }
    }

    pub const fn id(self) -> NodeId {
        self.id
    }
}

/// A typed source attribute whose value is supplied from outside the graph.
#[derive(Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StaticAttribute<T> {
    attribute: Attribute<T>,
}

impl<T> Clone for StaticAttribute<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for StaticAttribute<T> {}

impl<T> StaticAttribute<T> {
    const fn new(id: NodeId) -> Self {
        Self {
            attribute: Attribute::new(id),
        }
    }

    pub const fn id(self) -> NodeId {
        self.attribute.id()
    }

    pub const fn attribute(self) -> Attribute<T> {
        self.attribute
    }
}

impl<T> From<StaticAttribute<T>> for Attribute<T> {
    fn from(attribute: StaticAttribute<T>) -> Self {
        attribute.attribute
    }
}

/// A typed derived attribute whose value is produced by a rule.
#[derive(Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DynamicAttribute<T> {
    attribute: Attribute<T>,
}

impl<T> Clone for DynamicAttribute<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for DynamicAttribute<T> {}

impl<T> DynamicAttribute<T> {
    const fn new(id: NodeId) -> Self {
        Self {
            attribute: Attribute::new(id),
        }
    }

    pub const fn id(self) -> NodeId {
        self.attribute.id()
    }

    pub const fn attribute(self) -> Attribute<T> {
        self.attribute
    }
}

impl<T> From<DynamicAttribute<T>> for Attribute<T> {
    fn from(attribute: DynamicAttribute<T>) -> Self {
        attribute.attribute
    }
}

/// A small runtime type descriptor.
///
/// This deliberately does not use Rust's `TypeId`: `TypeId` is only meaningful
/// inside one Rust program, while this graph is being shaped so Swift or another
/// rule provider can participate. In a real bridge this could become a stable
/// ABI descriptor, metadata pointer, mangled type name, or host-owned type key.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TypeDescriptor {
    name: &'static str,
}

impl TypeDescriptor {
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    pub const fn name(self) -> &'static str {
        self.name
    }
}

/// Type-erased value storage for this first runtime pass.
///
/// The graph should not need to know whether a value came from Rust, Swift, or
/// some other host. So it stores a type descriptor plus bytes. This is still a
/// simplified model: a production graph would likely attach clone/drop/compare
/// callbacks to the type descriptor so non-trivial values can be owned safely and
/// compared efficiently.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValueStorage {
    value_type: TypeDescriptor,
    bytes: Vec<u8>,
    comparison: ValueComparison,
}

/// How the graph decides whether a recomputed value meaningfully changed.
///
/// `Bytewise` is enough for this first implementation because our current test
/// values are simple byte-backed scalars and static strings. A real Swift or
/// cross-language host would likely replace or extend this with a host-supplied
/// comparison callback on the type descriptor, so values can use Swift `Equatable`,
/// identity checks, bitwise comparison, or "always changed" semantics as needed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueComparison {
    Bytewise,
    AlwaysChanged,
}

impl ValueStorage {
    pub fn from_bytes(value_type: TypeDescriptor, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            value_type,
            bytes: bytes.into(),
            comparison: ValueComparison::Bytewise,
        }
    }

    pub fn with_comparison(mut self, comparison: ValueComparison) -> Self {
        self.comparison = comparison;
        self
    }

    pub fn from_bool(value: bool) -> Self {
        Self::from_bytes(TypeDescriptor::new("bool"), [u8::from(value)])
    }

    pub fn from_i64(value: i64) -> Self {
        Self::from_bytes(TypeDescriptor::new("i64"), value.to_ne_bytes())
    }

    pub fn from_static_str(value: &'static str) -> Self {
        Self::from_bytes(TypeDescriptor::new("&'static str"), value.as_bytes())
    }

    pub const fn value_type(&self) -> TypeDescriptor {
        self.value_type
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn comparison(&self) -> ValueComparison {
        self.comparison
    }

    pub fn meaningfully_changed_from(&self, old: Option<&ValueStorage>) -> bool {
        let Some(old) = old else {
            return true;
        };

        if self.value_type != old.value_type {
            return true;
        }

        match (old.comparison, self.comparison) {
            (ValueComparison::AlwaysChanged, _) | (_, ValueComparison::AlwaysChanged) => true,
            (ValueComparison::Bytewise, ValueComparison::Bytewise) => self.bytes != old.bytes,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        (self.value_type == TypeDescriptor::new("bool") && self.bytes.len() == 1)
            .then(|| self.bytes[0] != 0)
    }

    pub fn as_i64(&self) -> Option<i64> {
        if self.value_type != TypeDescriptor::new("i64") || self.bytes.len() != 8 {
            return None;
        }

        let mut bytes = [0; 8];
        bytes.copy_from_slice(&self.bytes);
        Some(i64::from_ne_bytes(bytes))
    }

    pub fn as_static_str(&self) -> Option<&str> {
        (self.value_type == TypeDescriptor::new("&'static str"))
            .then(|| std::str::from_utf8(&self.bytes).ok())
            .flatten()
    }
}

/// Rust-side values that can move through the typed Attribute API.
///
/// The graph still stores bytes internally so foreign hosts can plug in later,
/// but normal Rust callers should read and write `T` instead of manually building
/// `ValueStorage`.
pub trait AttributeValue: Clone + Sized + 'static {
    fn type_descriptor() -> TypeDescriptor;
    fn into_storage(self) -> ValueStorage;
    fn from_storage(storage: &ValueStorage) -> Option<Self>;
}

impl AttributeValue for bool {
    fn type_descriptor() -> TypeDescriptor {
        TypeDescriptor::new("bool")
    }

    fn into_storage(self) -> ValueStorage {
        ValueStorage::from_bool(self)
    }

    fn from_storage(storage: &ValueStorage) -> Option<Self> {
        storage.as_bool()
    }
}

impl AttributeValue for i64 {
    fn type_descriptor() -> TypeDescriptor {
        TypeDescriptor::new("i64")
    }

    fn into_storage(self) -> ValueStorage {
        ValueStorage::from_i64(self)
    }

    fn from_storage(storage: &ValueStorage) -> Option<Self> {
        storage.as_i64()
    }
}

impl AttributeValue for String {
    fn type_descriptor() -> TypeDescriptor {
        TypeDescriptor::new("String")
    }

    fn into_storage(self) -> ValueStorage {
        ValueStorage::from_bytes(Self::type_descriptor(), self.into_bytes())
    }

    fn from_storage(storage: &ValueStorage) -> Option<Self> {
        if storage.value_type() != Self::type_descriptor() {
            return None;
        }

        String::from_utf8(storage.bytes().to_vec()).ok()
    }
}

/// Opaque handle to a rule body owned by a rule provider.
///
/// The graph never interprets this value. A Rust test might use it as a boxed
/// pointer. A Swift bridge might use it as an index into a Swift-owned rule table
/// or as an opaque retained object pointer. The only function that should
/// understand the handle is the update callback stored beside it.
///
/// A rule body is semantically immutable once its descriptor is installed in the
/// graph. Dependency handles, constants that affect output, and update semantics
/// must not be changed behind the graph's back. Changing inputs that can affect
/// output or dependency selection belong in source attributes and should be read
/// through [`EvaluationContext`]. Non-semantic counters, diagnostics, or caches
/// may use interior mutation only when they cannot affect output or observed
/// dependencies. To change a rule definition, remove the derived node and create
/// a new one; downstream rules that stored its old [`NodeId`] must also be
/// rebuilt.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RuleHandle(usize);

impl RuleHandle {
    pub const fn from_raw(raw: usize) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> usize {
        self.0
    }
}

/// Function used to execute an opaque rule body.
///
/// This is intentionally a function pointer, not `Box<dyn Rule>`, because the
/// long-term model is "external rule provider supplies body plus updater." A
/// real Swift bridge would usually expose a C ABI trampoline that adapts Swift's
/// rule object to this Rust-side callback shape.
///
/// Returning an error aborts the current node's evaluation without committing
/// its new output or observed dependency set. The node remains dirty and can be
/// retried. If the callback unwinds with a panic, the graph removes the node's
/// evaluation frame and then resumes the same panic. A caller that catches the
/// unwind can therefore use the graph again, but the panic is not converted to
/// a [`GraphError`] or treated as a graph-wide rollback. With `panic=abort`, the
/// process terminates instead and no recovery is possible.
pub type UpdateFn = fn(RuleHandle, &mut EvaluationContext<'_>) -> Result<(), GraphError>;

/// Optional cleanup callback for the opaque rule body.
///
/// If an external host owns rule bodies elsewhere, this can be `None`. If the
/// graph is handed ownership of a boxed or retained body, provide a destroy
/// callback so removing/dropping the node releases it.
pub type DestroyFn = fn(RuleHandle);

/// Metadata and callbacks for a derived node's rule.
///
/// This is the core "external rules can plug in here" object. The graph stores
/// the body handle and callback, but it does not know the concrete rule type or
/// contain the rule logic. The descriptor and the rule body it identifies are
/// semantically immutable for the lifetime of the derived node. Runtime-varying
/// evaluation inputs must be modeled as source attributes.
#[derive(Debug)]
pub struct RuleDescriptor {
    body: RuleHandle,
    update: UpdateFn,
    destroy: Option<DestroyFn>,
    body_type: TypeDescriptor,
    value_type: TypeDescriptor,
    debug_name: &'static str,
}

impl RuleDescriptor {
    pub fn new(
        body: RuleHandle,
        update: UpdateFn,
        body_type: TypeDescriptor,
        value_type: TypeDescriptor,
        debug_name: &'static str,
    ) -> Self {
        Self {
            body,
            update,
            destroy: None,
            body_type,
            value_type,
            debug_name,
        }
    }

    pub fn with_destroy(mut self, destroy: DestroyFn) -> Self {
        self.destroy = Some(destroy);
        self
    }

    pub const fn body(&self) -> RuleHandle {
        self.body
    }

    pub const fn update(&self) -> UpdateFn {
        self.update
    }

    pub const fn body_type(&self) -> TypeDescriptor {
        self.body_type
    }

    pub const fn value_type(&self) -> TypeDescriptor {
        self.value_type
    }

    pub const fn debug_name(&self) -> &'static str {
        self.debug_name
    }
}

impl Drop for RuleDescriptor {
    fn drop(&mut self) {
        if let Some(destroy) = self.destroy {
            destroy(self.body);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    /// Externally supplied state.
    Source,
    /// Calculated from a rule and the attributes that rule reads.
    Derived,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeState {
    /// The cached value is current after the node's dependencies have been validated.
    Clean,
    /// The cached value may be stale and should be recomputed before use.
    Dirty,
    /// A transitive dependency may be stale, so reads must validate inputs first.
    ///
    /// This is the key lazy AttributeGraph state: when `A -> B -> C` and `A`
    /// changes, `B` is definitely dirty but `C` is only maybe dirty. Reading `C`
    /// first validates `B`; if `B` recomputes to the same value, `C` can stay
    /// clean without running its own rule.
    MaybeDirty,
}

/// Internal node storage.
///
/// Source nodes have a value and no rule. Derived nodes have an optional cached
/// value plus a `RuleDescriptor`. Active dependencies are stored on the
/// dependent node so they can be replaced after each evaluation, which is what
/// eventually makes conditional dependencies work.
#[derive(Debug)]
pub struct Node {
    id: NodeId,
    kind: NodeKind,
    state: NodeState,
    value: Option<ValueStorage>,
    active_dependencies: HashSet<NodeId>,
    rule: Option<RuleDescriptor>,
}

impl Node {
    pub const fn id(&self) -> NodeId {
        self.id
    }

    pub const fn kind(&self) -> NodeKind {
        self.kind
    }

    pub const fn state(&self) -> NodeState {
        self.state
    }

    pub const fn is_dirty(&self) -> bool {
        matches!(self.state, NodeState::Dirty)
    }

    #[doc(hidden)]
    pub fn debug_cached_value(&self) -> Option<&ValueStorage> {
        self.value.as_ref()
    }

    pub fn value_type(&self) -> Option<TypeDescriptor> {
        match self.kind {
            NodeKind::Source => self.value.as_ref().map(ValueStorage::value_type),
            NodeKind::Derived => self.rule.as_ref().map(RuleDescriptor::value_type),
        }
    }

    pub fn rule(&self) -> Option<&RuleDescriptor> {
        self.rule.as_ref()
    }

    pub fn active_dependencies(&self) -> Vec<NodeId> {
        sorted_ids(Some(&self.active_dependencies))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Edge {
    // `dependent` depends on `dependency`.
    // If `dependency` changes, `dependent` may need to be recomputed.
    pub dependency: NodeId,
    pub dependent: NodeId,
}

impl Edge {
    pub const fn new(dependency: NodeId, dependent: NodeId) -> Self {
        Self {
            dependency,
            dependent,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EdgeState {
    Inactive,
    Settled,
    Pending,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DependencyChangeSet {
    pub added: Vec<NodeId>,
    pub removed: Vec<NodeId>,
    pub retained: Vec<NodeId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UpdateOutcome {
    pub dependency_changes: DependencyChangeSet,
    pub value_changed: bool,
    pub dirtied_dependents: Vec<NodeId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphError {
    GraphMismatch {
        expected: GraphId,
        actual: GraphId,
    },
    MissingNode(NodeId),
    MissingValue(NodeId),
    MissingOutput(NodeId),
    NotSource(NodeId),
    NotDerived(NodeId),
    SelfDependency(NodeId),
    CycleDetected,
    RuleValueTypeMismatch {
        expected: TypeDescriptor,
        actual: TypeDescriptor,
    },
    ValueTypeMismatch {
        node: NodeId,
        expected: TypeDescriptor,
        actual: TypeDescriptor,
    },
    ValueDecodeFailed {
        node: NodeId,
        value_type: TypeDescriptor,
    },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GraphMismatch { expected, actual } => write!(
                f,
                "node belongs to graph {actual}, but this operation uses graph {expected}"
            ),
            Self::MissingNode(id) => write!(f, "missing node {id}"),
            Self::MissingValue(id) => write!(f, "node {id} has no cached value"),
            Self::MissingOutput(id) => {
                write!(f, "rule for node {id} did not set an output value")
            }
            Self::NotSource(id) => write!(f, "node {id} is not a source node"),
            Self::NotDerived(id) => write!(f, "node {id} is not a derived node"),
            Self::SelfDependency(id) => write!(f, "node {id} cannot depend on itself"),
            Self::CycleDetected => write!(f, "dependency cycle detected"),
            Self::RuleValueTypeMismatch { expected, actual } => write!(
                f,
                "rule expected to produce value type {}, got {}",
                expected.name(),
                actual.name()
            ),
            Self::ValueTypeMismatch {
                node,
                expected,
                actual,
            } => write!(
                f,
                "node {} expected value type {}, got {}",
                node,
                expected.name(),
                actual.name()
            ),
            Self::ValueDecodeFailed { node, value_type } => write!(
                f,
                "node {} has invalid cached bytes for value type {}",
                node,
                value_type.name()
            ),
        }
    }
}

impl Error for GraphError {}

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
    /// [`ValueComparison::AlwaysChanged`] always invalidate.
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

fn next_graph_id() -> GraphId {
    NEXT_GRAPH_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map(GraphId)
        .expect("attribute graph exhausted its graph id space")
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

fn decode_attribute_value<T: AttributeValue>(
    id: NodeId,
    value: &ValueStorage,
) -> Result<T, GraphError> {
    let expected = T::type_descriptor();
    let actual = value.value_type();

    if expected != actual {
        return Err(GraphError::ValueTypeMismatch {
            node: id,
            expected,
            actual,
        });
    }

    T::from_storage(value).ok_or(GraphError::ValueDecodeFailed {
        node: id,
        value_type: expected,
    })
}

fn sorted_ids(ids: Option<&HashSet<NodeId>>) -> Vec<NodeId> {
    let mut ids = ids.into_iter().flatten().copied().collect::<Vec<NodeId>>();
    ids.sort();
    ids
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
