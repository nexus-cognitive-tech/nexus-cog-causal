//! Causal graph engine, persisted via [`nexus-cog-storage`].
//!
//! The SQLite table is the source of truth; the in-memory `petgraph` is a
//! derived cache rebuilt from `backend` on every `with_backend` call. All
//! mutating operations write to SQL first, then update the in-memory graph
//! from the returned row.

use std::sync::Arc;

use indexmap::IndexMap;
use nexus_cog_core::causal::{CausalEdge, CausalEdgeType, CausalGraph, CausalNode};
use nexus_cog_storage::{PersistenceBackend, SqliteBackend, StorageResult};
use parking_lot::RwLock;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;

/// Inner mutable state of [`CausalGraphEngine`].
#[derive(Debug)]
struct Inner {
    graph: DiGraph<CausalNode, CausalEdge>,
    index: IndexMap<String, NodeIndex>,
}

/// The causal graph engine. Every persistent engine is constructed against
/// the shared [`PersistenceBackend`].
#[derive(Clone)]
pub struct CausalGraphEngine {
    backend: Arc<dyn PersistenceBackend>,
    inner: Arc<RwLock<Inner>>,
}

impl std::fmt::Debug for CausalGraphEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CausalGraphEngine")
            .field("backend", &self.backend.describe())
            .field("nodes", &self.node_count())
            .field("edges", &self.edge_count())
            .finish()
    }
}

impl CausalGraphEngine {
    /// Construct an engine backed by `backend`. The schema is applied
    /// idempotently and every persisted node / edge is loaded into the
    /// in-memory cache before the constructor returns.
    pub fn with_backend(backend: Arc<dyn PersistenceBackend>) -> StorageResult<Self> {
        super::schema::register(backend.as_ref())?;
        let (nodes, edges) = (super::schema::load_all_nodes(backend.as_ref())?, super::schema::load_all_edges(backend.as_ref())?);
        let mut graph: DiGraph<CausalNode, CausalEdge> = DiGraph::new();
        let mut index: IndexMap<String, NodeIndex> = IndexMap::new();
        for n in nodes {
            let id = n.id.clone();
            let idx = graph.add_node(n);
            index.insert(id, idx);
        }
        for e in edges {
            let Some(&from) = index.get(&e.from) else { continue };
            let Some(&to) = index.get(&e.to) else { continue };
            if let Some(existing) = graph.find_edge(from, to) {
                graph.remove_edge(existing);
            }
            graph.add_edge(from, to, e);
        }
        Ok(Self {
            backend,
            inner: Arc::new(RwLock::new(Inner { graph, index })),
        })
    }

    /// Convenience constructor that opens an in-memory SQLite database and
    /// uses it. Useful for tests and one-off scripts.
    pub fn in_memory() -> StorageResult<Self> {
        Self::with_backend(Arc::new(SqliteBackend::open_in_memory()?))
    }

    /// Backend description.
    #[must_use]
    pub fn backend_info(&self) -> String {
        self.backend.describe()
    }

    /// Number of nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.inner.read().graph.node_count()
    }

    /// Number of edges.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.inner.read().graph.edge_count()
    }

    /// Add a node. Persists to backend first; if persistence fails the
    /// in-memory state is unchanged.
    pub fn add_node(&self, node: CausalNode) -> StorageResult<NodeIndex> {
        super::schema::upsert_node(self.backend.as_ref(), &node)?;
        let id = node.id.clone();
        let mut inner = self.inner.write();
        let idx = inner.graph.add_node(node);
        inner.index.insert(id, idx);
        Ok(idx)
    }

    /// Add an edge between two existing nodes. Persists first; in-memory
    /// cache is updated only after the SQL write succeeds. Returns `false`
    /// if either endpoint is missing.
    pub fn add_edge(&self, edge: CausalEdge) -> StorageResult<bool> {
        {
            let inner = self.inner.read();
            if inner.index.get(&edge.from).is_none() || inner.index.get(&edge.to).is_none() {
                return Ok(false);
            }
        }
        super::schema::upsert_edge(self.backend.as_ref(), &edge)?;
        let mut inner = self.inner.write();
        let Some(&from) = inner.index.get(&edge.from) else { return Ok(false) };
        let Some(&to) = inner.index.get(&edge.to) else { return Ok(false) };
        if let Some(existing) = inner.graph.find_edge(from, to) {
            inner.graph.remove_edge(existing);
        }
        inner.graph.add_edge(from, to, edge);
        Ok(true)
    }

    /// Get a node by ID.
    #[must_use]
    pub fn node(&self, id: &str) -> Option<CausalNode> {
        self.inner.read().index.get(id).and_then(|idx| self.inner.read().graph.node_weight(*idx).cloned())
    }

    /// All nodes of a particular type.
    #[must_use]
    pub fn nodes_of_type(&self, ty: nexus_cog_core::causal::CausalNodeType) -> Vec<CausalNode> {
        self.inner
            .read()
            .graph
            .node_indices()
            .filter_map(|idx| self.inner.read().graph.node_weight(idx).cloned())
            .filter(|n| n.node_type == ty)
            .collect()
    }

    /// All nodes.
    #[must_use]
    pub fn nodes(&self) -> Vec<CausalNode> {
        self.inner
            .read()
            .graph
            .node_indices()
            .filter_map(|idx| self.inner.read().graph.node_weight(idx).cloned())
            .collect()
    }

    /// All edges.
    #[must_use]
    pub fn edges(&self) -> Vec<CausalEdge> {
        self.inner
            .read()
            .graph
            .edge_references()
            .map(|e| e.weight().clone())
            .collect()
    }

    /// Forward closure from a node (all descendants).
    #[must_use]
    pub fn forward_closure(&self, id: &str) -> std::collections::HashSet<String> {
        let inner = self.inner.read();
        let Some(&start) = inner.index.get(id) else { return std::collections::HashSet::new() };
        let mut visited = std::collections::HashSet::new();
        let mut order = Vec::new();
        let mut stack = vec![start];
        while let Some(idx) = stack.pop() {
            if !visited.insert(idx) {
                continue;
            }
            order.push(idx);
            for edge in inner.graph.edges_directed(idx, petgraph::Direction::Outgoing) {
                stack.push(edge.target());
            }
        }
        order
            .into_iter()
            .filter_map(|idx| inner.graph.node_weight(idx))
            .map(|n| n.id.clone())
            .collect()
    }

    /// Backward closure from a node (all ancestors).
    #[must_use]
    pub fn backward_closure(&self, id: &str) -> std::collections::HashSet<String> {
        let inner = self.inner.read();
        let Some(&start) = inner.index.get(id) else { return std::collections::HashSet::new() };
        let mut visited = std::collections::HashSet::new();
        let mut order = Vec::new();
        let mut stack = vec![start];
        while let Some(idx) = stack.pop() {
            if !visited.insert(idx) {
                continue;
            }
            order.push(idx);
            for edge in inner.graph.edges_directed(idx, petgraph::Direction::Incoming) {
                stack.push(edge.source());
            }
        }
        order
            .into_iter()
            .filter_map(|idx| inner.graph.node_weight(idx))
            .map(|n| n.id.clone())
            .collect()
    }

    /// Snapshot the in-memory cache to a serializable graph.
    #[must_use]
    pub fn snapshot(&self) -> CausalGraph {
        let inner = self.inner.read();
        let nodes: Vec<CausalNode> = inner
            .graph
            .node_indices()
            .filter_map(|idx| inner.graph.node_weight(idx).cloned())
            .collect();
        let edges: Vec<CausalEdge> = inner
            .graph
            .edge_references()
            .map(|e| e.weight().clone())
            .collect();
        CausalGraph {
            nodes,
            edges,
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now(),
            scope: String::new(),
        }
    }

    /// Look up the internal node index for a node ID.
    #[must_use]
    pub fn index_of(&self, id: &str) -> Option<NodeIndex> {
        self.inner.read().index.get(id).copied()
    }

    /// Iterator over internal node indices.
    #[must_use]
    pub fn node_indices(&self) -> Vec<NodeIndex> {
        self.inner.read().graph.node_indices().collect()
    }

    /// Get the weight (node) at a given index.
    #[must_use]
    pub fn graph_weight(&self, idx: NodeIndex) -> Option<CausalNode> {
        self.inner.read().graph.node_weight(idx).cloned()
    }

    /// Get immediate parent node indices of the given node index.
    #[must_use]
    pub fn immediate_parents(&self, idx: NodeIndex) -> Vec<NodeIndex> {
        self.inner
            .read()
            .graph
            .edges_directed(idx, petgraph::Direction::Incoming)
            .map(|e| e.source())
            .collect()
    }

    /// Get immediate child node indices of the given node index.
    #[must_use]
    pub fn immediate_children(&self, idx: NodeIndex) -> Vec<NodeIndex> {
        self.inner
            .read()
            .graph
            .edges_directed(idx, petgraph::Direction::Outgoing)
            .map(|e| e.target())
            .collect()
    }

    /// Acquire a read guard for advanced traversal.
    #[must_use]
    pub fn read(&self) -> CausalGraphReadGuard<'_> {
        CausalGraphReadGuard { inner: self.inner.read() }
    }
}

/// Read-only guard over a [`CausalGraphEngine`] for advanced traversal.
pub struct CausalGraphReadGuard<'a> {
    inner: parking_lot::RwLockReadGuard<'a, Inner>,
}

impl<'a> CausalGraphReadGuard<'a> {
    /// Direct access to the underlying petgraph.
    #[must_use]
    pub fn graph(&self) -> &DiGraph<CausalNode, CausalEdge> {
        &self.inner.graph
    }

    /// Direct access to the id → index map.
    #[must_use]
    pub fn index(&self) -> &IndexMap<String, NodeIndex> {
        &self.inner.index
    }

    /// Look up the internal node index for a node ID.
    #[must_use]
    pub fn index_of(&self, id: &str) -> Option<NodeIndex> {
        self.inner.index.get(id).copied()
    }

    /// Iterator over internal node indices.
    #[must_use]
    pub fn node_indices(&self) -> impl Iterator<Item = NodeIndex> + '_ {
        self.inner.graph.node_indices()
    }

    /// Get the weight (node) at a given index.
    #[must_use]
    pub fn graph_weight(&self, idx: NodeIndex) -> Option<&CausalNode> {
        self.inner.graph.node_weight(idx)
    }
}

// Silence "unused import" warnings for traits we keep around for callers.
#[allow(unused_imports)]
use CausalEdgeType as _;

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_cog_core::causal::{CausalNodeType, CausalEdge, CausalEdgeType};
    use nexus_cog_core::common::Confidence;

    fn node(id: &str) -> CausalNode {
        CausalNode {
            id: id.into(),
            node_type: CausalNodeType::CodeEntity,
            name: id.into(),
            description: String::new(),
            file: None,
            line: None,
            confidence: Confidence::new(1.0),
            tags: vec![],
        }
    }

    #[test]
    fn add_node_persists() {
        let e = CausalGraphEngine::in_memory().unwrap();
        e.add_node(node("a")).unwrap();
        let snap = e.snapshot();
        assert_eq!(snap.nodes.len(), 1);
    }

    #[test]
    fn add_edge_with_real_edge_type() {
        let e = CausalGraphEngine::in_memory().unwrap();
        e.add_node(node("a")).unwrap();
        e.add_node(node("b")).unwrap();
        e.add_edge(CausalEdge {
            from: "a".into(),
            to: "b".into(),
            edge_type: CausalEdgeType::Mitigates,
            strength: 0.42,
            confidence: Confidence::new(1.0),
            evidence: vec!["e1".into()],
        }).unwrap();
        let edges = e.edges();
        assert_eq!(edges[0].edge_type, CausalEdgeType::Mitigates);
        assert_eq!(edges[0].strength, 0.42);
        assert_eq!(edges[0].evidence, vec!["e1".to_string()]);
    }

    #[test]
    fn persists_across_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("causal.db");
        {
            let backend = Arc::new(SqliteBackend::open(&path).unwrap());
            let e = CausalGraphEngine::with_backend(backend).unwrap();
            e.add_node(node("a")).unwrap();
            e.add_node(node("b")).unwrap();
            e.add_edge(CausalEdge {
                from: "a".into(),
                to: "b".into(),
                edge_type: CausalEdgeType::Causes,
                strength: 0.5,
                confidence: Confidence::new(1.0),
                evidence: vec![],
            }).unwrap();
        }
        let backend = Arc::new(SqliteBackend::open(&path).unwrap());
        let e = CausalGraphEngine::with_backend(backend).unwrap();
        assert_eq!(e.node_count(), 2);
        assert_eq!(e.edge_count(), 1);
        let edges = e.edges();
        assert_eq!(edges[0].edge_type, CausalEdgeType::Causes);
    }
}
