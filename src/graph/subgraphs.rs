use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use super::AttributeGraph;
use crate::error::GraphError;
use crate::identity::{NodeId, SubgraphId};
use crate::subgraph::{Subgraph, SubgraphRemoval};

impl AttributeGraph {
    /// Create an empty ownership subgraph.
    ///
    /// Subgraphs are lifetime regions inside this graph, not independent
    /// evaluation graphs. Dependencies may freely cross their boundaries. A
    /// parent is used only for recursive lifetime ownership and cannot be
    /// changed after creation. A stale same-graph parent returns
    /// [`GraphError::MissingSubgraph`]; a parent from another graph returns
    /// [`GraphError::GraphMismatch`].
    pub fn create_subgraph(
        &mut self,
        parent: Option<SubgraphId>,
    ) -> Result<SubgraphId, GraphError> {
        if let Some(parent) = parent {
            self.ensure_subgraph(parent)?;
        }

        let id = self.next_subgraph_id();
        self.subgraphs.insert(
            id,
            Subgraph {
                id,
                parent,
                children: HashSet::new(),
                nodes: HashSet::new(),
            },
        );

        if let Some(parent) = parent {
            self.subgraphs
                .get_mut(&parent)
                .expect("validated parent subgraph should exist")
                .children
                .insert(id);
        }

        Ok(id)
    }

    /// Run construction with `id` as the owner of every node created inside it.
    ///
    /// Helper functions do not need to receive the subgraph id: the existing
    /// node-creation APIs register nodes with the innermost active construction
    /// subgraph. The previous construction context is restored after a normal
    /// return or an unwinding panic. This method does not roll back nodes when
    /// the closure returns an application-level error; use
    /// [`Self::build_subgraph`] when a newly created subgraph should be
    /// transactional. Stale and foreign ids are rejected before the callback
    /// runs.
    pub fn with_subgraph<R>(
        &mut self,
        id: SubgraphId,
        build: impl FnOnce(&mut AttributeGraph) -> R,
    ) -> Result<R, GraphError> {
        self.ensure_subgraph(id)?;
        self.construction_subgraphs.push(id);

        let result = catch_unwind(AssertUnwindSafe(|| build(self)));
        let popped = self.construction_subgraphs.pop();
        debug_assert_eq!(popped, Some(id));

        match result {
            Ok(value) => Ok(value),
            Err(payload) => resume_unwind(payload),
        }
    }

    /// Create and transactionally build one subgraph.
    ///
    /// A callback error removes the new subgraph and everything created inside
    /// it before returning the error. An unwinding callback panic performs the
    /// same cleanup and then resumes the original panic. Existing parent and
    /// sibling subgraphs are not affected. With `panic=abort`, the process ends
    /// and no rollback can run.
    pub fn build_subgraph<R>(
        &mut self,
        parent: Option<SubgraphId>,
        build: impl FnOnce(&mut AttributeGraph, SubgraphId) -> Result<R, GraphError>,
    ) -> Result<(SubgraphId, R), GraphError> {
        let id = self.create_subgraph(parent)?;
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.with_subgraph(id, |graph| build(graph, id))
        }));

        match result {
            Ok(Ok(Ok(value))) => Ok((id, value)),
            Ok(Ok(Err(error))) | Ok(Err(error)) => {
                self.remove_subgraph(id)?;
                Err(error)
            }
            Err(payload) => {
                self.remove_subgraph(id)
                    .expect("a panicking subgraph build should be removable after scope cleanup");
                resume_unwind(payload)
            }
        }
    }

    /// Return the innermost subgraph currently receiving newly created nodes.
    pub fn current_subgraph(&self) -> Option<SubgraphId> {
        self.construction_subgraphs.last().copied()
    }

    pub fn subgraph(&self, id: SubgraphId) -> Option<&Subgraph> {
        self.subgraphs.get(&id)
    }

    pub fn contains_subgraph(&self, id: SubgraphId) -> bool {
        self.subgraphs.contains_key(&id)
    }

    pub fn subgraph_count(&self) -> usize {
        self.subgraphs.len()
    }

    pub fn subgraphs(&self) -> Vec<SubgraphId> {
        let mut ids = self.subgraphs.keys().copied().collect::<Vec<_>>();
        ids.sort();
        ids
    }

    /// Permanently remove a subgraph, all descendant subgraphs, and every node
    /// they own as one externally atomic mutation.
    ///
    /// Surviving active dependents are invalidated before edges are detached.
    /// All structural removal completes before owned rule descriptors are
    /// dropped, so destroy callbacks observe no partially attached subtree. A
    /// stale same-graph id returns [`GraphError::MissingSubgraph`], a foreign id
    /// returns [`GraphError::GraphMismatch`], and an active subtree returns
    /// [`GraphError::SubgraphInUse`]. The in-use error identifies the innermost
    /// active descendant encountered.
    pub fn remove_subgraph(&mut self, id: SubgraphId) -> Result<SubgraphRemoval, GraphError> {
        self.ensure_subgraph(id)?;

        let subgraph_postorder = self.subgraph_postorder(id);
        let subgraph_set = subgraph_postorder.iter().copied().collect::<HashSet<_>>();

        if let Some(active) = self
            .construction_subgraphs
            .iter()
            .rev()
            .find(|active| subgraph_set.contains(active))
        {
            return Err(GraphError::SubgraphInUse(*active));
        }

        let removed_node_set = subgraph_postorder
            .iter()
            .flat_map(|subgraph| {
                self.subgraphs
                    .get(subgraph)
                    .expect("collected subgraph should exist")
                    .nodes
                    .iter()
            })
            .copied()
            .collect::<HashSet<_>>();

        let mut dirtied_dependents = removed_node_set
            .iter()
            .flat_map(|node| self.dependents.get(node).into_iter().flatten())
            .filter(|dependent| !removed_node_set.contains(dependent))
            .copied()
            .collect::<Vec<_>>();
        dirtied_dependents.sort();
        dirtied_dependents.dedup();

        let mut removal_order = self.topological_order()?;
        removal_order.retain(|node| removed_node_set.contains(node));
        removal_order.reverse();

        let mut removed_nodes = Vec::with_capacity(removal_order.len());
        for node in removal_order {
            removed_nodes.push(
                self.remove_node(node)
                    .expect("a node collected for subgraph removal should exist"),
            );
        }

        for subgraph in &subgraph_postorder {
            let removed = self
                .subgraphs
                .remove(subgraph)
                .expect("a collected subgraph should exist");

            if let Some(parent) = removed.parent
                && let Some(parent) = self.subgraphs.get_mut(&parent)
            {
                parent.children.remove(subgraph);
            }
        }

        let mut removed_subgraphs = subgraph_postorder;
        removed_subgraphs.sort();
        let mut removed_node_ids = removed_node_set.into_iter().collect::<Vec<_>>();
        removed_node_ids.sort();

        for node in removed_nodes {
            drop(node);
        }

        Ok(SubgraphRemoval {
            subgraphs: removed_subgraphs,
            nodes: removed_node_ids,
            dirtied_dependents,
        })
    }

    fn ensure_subgraph(&self, id: SubgraphId) -> Result<(), GraphError> {
        if id.graph_id() != self.id {
            return Err(GraphError::GraphMismatch {
                expected: self.id,
                actual: id.graph_id(),
            });
        }

        if self.subgraphs.contains_key(&id) {
            Ok(())
        } else {
            Err(GraphError::MissingSubgraph(id))
        }
    }

    pub(super) fn register_node_with_current_subgraph(&mut self, id: NodeId) -> Option<SubgraphId> {
        let subgraph = self.current_subgraph()?;
        self.subgraphs
            .get_mut(&subgraph)
            .expect("active construction subgraph should exist")
            .nodes
            .insert(id);
        Some(subgraph)
    }

    fn subgraph_postorder(&self, root: SubgraphId) -> Vec<SubgraphId> {
        let mut order = Vec::new();
        let mut stack = vec![(root, false)];

        while let Some((id, expanded)) = stack.pop() {
            if expanded {
                order.push(id);
                continue;
            }

            stack.push((id, true));
            let mut children = self
                .subgraphs
                .get(&id)
                .expect("subgraph traversal should only contain live ids")
                .children
                .iter()
                .copied()
                .collect::<Vec<_>>();
            children.sort_by(|lhs, rhs| rhs.cmp(lhs));
            stack.extend(children.into_iter().map(|child| (child, false)));
        }

        order
    }

    fn next_subgraph_id(&mut self) -> SubgraphId {
        let id = SubgraphId::new(self.id, self.next_subgraph_id);
        self.next_subgraph_id = self
            .next_subgraph_id
            .checked_add(1)
            .expect("attribute graph exhausted its subgraph id space");
        id
    }
}
