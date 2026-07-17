use crate::error::GraphError;
use crate::graph::EvaluationContext;
use crate::value::TypeDescriptor;

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
/// a new one; downstream rules that stored its old [`crate::NodeId`] must also be
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
