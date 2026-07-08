//! SQL schema for the causal graph engine.
//!
//! All persistence goes through generic [`PersistenceBackend`] primitives.
//! This module owns every SQL statement that targets causal tables — no
//! other file in this crate (or in any other engine) writes SQL against
//! `causal_*` tables.

use nexus_cog_core::causal::{
    CausalEdge, CausalEdgeType, CausalGraph, CausalNode, CausalNodeType,
};
use nexus_cog_storage::{PersistenceBackend, SqlValue, StorageResult};

/// Migration owner used in `engine_migrations`.
pub const OWNER: &str = "nexus_cog_causal";
/// Current schema version.
pub const SCHEMA_VERSION: i32 = 1;

/// Canonical schema SQL. Idempotent.
pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS causal_nodes (
    id          TEXT PRIMARY KEY,
    node_type   TEXT NOT NULL,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    file        TEXT,
    line        INTEGER,
    confidence  REAL NOT NULL DEFAULT 1.0,
    tags        TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS causal_edges (
    from_node   TEXT NOT NULL REFERENCES causal_nodes(id) ON DELETE CASCADE,
    to_node     TEXT NOT NULL REFERENCES causal_nodes(id) ON DELETE CASCADE,
    edge_type   TEXT NOT NULL,
    strength    REAL NOT NULL DEFAULT 0.5,
    confidence  REAL NOT NULL DEFAULT 1.0,
    evidence    TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (from_node, to_node),
    CHECK (from_node <> to_node)
);

CREATE INDEX IF NOT EXISTS causal_edges_to_idx ON causal_edges(to_node);
CREATE INDEX IF NOT EXISTS causal_edges_type_idx ON causal_edges(edge_type);
"#;

/// Register the schema with `backend`. Idempotent.
pub fn register(backend: &dyn PersistenceBackend) -> StorageResult<()> {
    backend.apply_migrations(OWNER, SCHEMA_VERSION, SCHEMA_SQL)
}

/// Insert (or replace) a node.
pub fn upsert_node(backend: &dyn PersistenceBackend, node: &CausalNode) -> StorageResult<()> {
    backend.exec(
        "INSERT INTO causal_nodes (id, node_type, name, description, file, line, confidence, tags) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT(id) DO UPDATE SET \
             node_type = excluded.node_type, \
             name = excluded.name, \
             description = excluded.description, \
             file = excluded.file, \
             line = excluded.line, \
             confidence = excluded.confidence, \
             tags = excluded.tags",
        &[
            SqlValue::text(&node.id),
            SqlValue::text(node_type_id(node.node_type)),
            SqlValue::text(&node.name),
            SqlValue::text(&node.description),
            node.file.as_ref().map_or(SqlValue::Null, SqlValue::text),
            node.line.map(|n| SqlValue::int(n as i64)).unwrap_or(SqlValue::Null),
            SqlValue::real(node.confidence.value() as f64),
            SqlValue::text(node.tags.join(",")),
        ],
    )?;
    Ok(())
}

/// Insert (or replace) an edge.
pub fn upsert_edge(backend: &dyn PersistenceBackend, edge: &CausalEdge) -> StorageResult<()> {
    backend.exec(
        "INSERT INTO causal_edges (from_node, to_node, edge_type, strength, confidence, evidence) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(from_node, to_node) DO UPDATE SET \
             edge_type = excluded.edge_type, \
             strength = excluded.strength, \
             confidence = excluded.confidence, \
             evidence = excluded.evidence",
        &[
            SqlValue::text(&edge.from),
            SqlValue::text(&edge.to),
            SqlValue::text(edge_type_id(edge.edge_type)),
            SqlValue::real(edge.strength as f64),
            SqlValue::real(edge.confidence.value() as f64),
            SqlValue::text(edge.evidence.join("\n")),
        ],
    )?;
    Ok(())
}

/// Load every node. Returns `(node, has_edge_target_marker)`.
pub fn load_all_nodes(backend: &dyn PersistenceBackend) -> StorageResult<Vec<CausalNode>> {
    let rows = backend.fetch_all(
        "SELECT id, node_type, name, description, file, line, confidence, tags \
         FROM causal_nodes ORDER BY id",
        &[],
    )?;
    rows.into_iter().map(row_to_node).collect()
}

/// Load every edge.
pub fn load_all_edges(backend: &dyn PersistenceBackend) -> StorageResult<Vec<CausalEdge>> {
    let rows = backend.fetch_all(
        "SELECT from_node, to_node, edge_type, strength, confidence, evidence \
         FROM causal_edges ORDER BY from_node, to_node",
        &[],
    )?;
    rows.into_iter().map(row_to_edge).collect()
}

fn row_to_node(row: Vec<SqlValue>) -> StorageResult<CausalNode> {
    let get_str = |i: usize| row.get(i).and_then(|v| v.as_str()).map(String::from);
    let get_f64 = |i: usize| row.get(i).and_then(|v| v.as_f64()).unwrap_or(1.0);
    let get_i64 = |i: usize| row.get(i).and_then(|v| v.as_i64());
    let tags_str = get_str(7).unwrap_or_default();
    let tags: Vec<String> = if tags_str.is_empty() {
        Vec::new()
    } else {
        tags_str.split(',').map(|s| s.to_string()).collect()
    };
    Ok(CausalNode {
        id: get_str(0).unwrap_or_default(),
        node_type: parse_node_type(&get_str(1).unwrap_or_else(|| "code_entity".into()))
            .unwrap_or(CausalNodeType::CodeEntity),
        name: get_str(2).unwrap_or_default(),
        description: get_str(3).unwrap_or_default(),
        file: get_str(4).filter(|s| !s.is_empty()),
        line: get_i64(5).and_then(|n| u32::try_from(n).ok()),
        confidence: nexus_cog_core::common::Confidence::new(get_f64(6) as f32),
        tags,
    })
}

fn row_to_edge(row: Vec<SqlValue>) -> StorageResult<CausalEdge> {
    let get_str = |i: usize| row.get(i).and_then(|v| v.as_str()).map(String::from);
    let get_f64 = |i: usize| row.get(i).and_then(|v| v.as_f64()).unwrap_or(0.5);
    let evidence_str = get_str(5).unwrap_or_default();
    let evidence: Vec<String> = if evidence_str.is_empty() {
        Vec::new()
    } else {
        evidence_str.lines().map(|s| s.to_string()).collect()
    };
    Ok(CausalEdge {
        from: get_str(0).unwrap_or_default(),
        to: get_str(1).unwrap_or_default(),
        edge_type: parse_edge_type(&get_str(2).unwrap_or_else(|| "causes".into()))
            .unwrap_or(CausalEdgeType::Causes),
        strength: get_f64(3) as f32,
        confidence: nexus_cog_core::common::Confidence::new(get_f64(4) as f32),
        evidence,
    })
}

fn node_type_id(t: CausalNodeType) -> &'static str {
    use CausalNodeType::*;
    match t {
        CodeEntity => "code_entity",
        Behavior => "behavior",
        Feature => "feature",
        Invariant => "invariant",
        Assumption => "assumption",
        Decision => "decision",
        Constraint => "constraint",
        Bug => "bug",
        ExternalDep => "external_dep",
    }
}

fn parse_node_type(s: &str) -> Option<CausalNodeType> {
    use CausalNodeType::*;
    Some(match s {
        "code_entity" | "code-entity" | "code" => CodeEntity,
        "behavior" => Behavior,
        "feature" => Feature,
        "invariant" => Invariant,
        "assumption" => Assumption,
        "decision" => Decision,
        "constraint" => Constraint,
        "bug" => Bug,
        "external_dep" | "external-dep" | "external" => ExternalDep,
        _ => return None,
    })
}

fn edge_type_id(t: CausalEdgeType) -> &'static str {
    use CausalEdgeType::*;
    match t {
        Causes => "causes",
        Enables => "enables",
        Prevents => "prevents",
        Mitigates => "mitigates",
        Correlates => "correlates",
    }
}

fn parse_edge_type(s: &str) -> Option<CausalEdgeType> {
    use CausalEdgeType::*;
    Some(match s {
        "causes" => Causes,
        "enables" => Enables,
        "prevents" => Prevents,
        "mitigates" => Mitigates,
        "correlates" => Correlates,
        _ => return None,
    })
}

/// Dump the entire graph as a [`CausalGraph`] snapshot.
pub fn snapshot(
    backend: &dyn PersistenceBackend,
) -> StorageResult<CausalGraph> {
    let nodes = load_all_nodes(backend)?;
    let edges = load_all_edges(backend)?;
    Ok(CausalGraph {
        nodes,
        edges,
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        scope: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_cog_storage::SqliteBackend;
    use std::sync::Arc;

    fn backend() -> Arc<SqliteBackend> {
        let b = Arc::new(SqliteBackend::open_in_memory().unwrap());
        register(b.as_ref()).unwrap();
        b
    }

    fn node(id: &str) -> CausalNode {
        CausalNode {
            id: id.into(),
            node_type: CausalNodeType::Bug,
            name: id.into(),
            description: "d".into(),
            file: None,
            line: None,
            confidence: nexus_cog_core::common::Confidence::new(1.0),
            tags: vec![],
        }
    }

    #[test]
    fn roundtrip_nodes_and_edges() {
        let b = backend();
        upsert_node(b.as_ref(), &node("a")).unwrap();
        upsert_node(b.as_ref(), &node("b")).unwrap();
        let edge = CausalEdge {
            from: "a".into(),
            to: "b".into(),
            edge_type: CausalEdgeType::Enables,
            strength: 0.7,
            confidence: nexus_cog_core::common::Confidence::new(0.9),
            evidence: vec!["e1".into()],
        };
        upsert_edge(b.as_ref(), &edge).unwrap();

        let snap = snapshot(b.as_ref()).unwrap();
        assert_eq!(snap.nodes.len(), 2);
        assert_eq!(snap.edges.len(), 1);
        assert_eq!(snap.edges[0].edge_type, CausalEdgeType::Enables);
        assert_eq!(snap.edges[0].strength, 0.7);
    }

    #[test]
    fn replace_edge_keeps_unique() {
        let b = backend();
        upsert_node(b.as_ref(), &node("a")).unwrap();
        upsert_node(b.as_ref(), &node("b")).unwrap();
        let e1 = CausalEdge {
            from: "a".into(),
            to: "b".into(),
            edge_type: CausalEdgeType::Causes,
            strength: 0.3,
            confidence: nexus_cog_core::common::Confidence::new(1.0),
            evidence: vec![],
        };
        let e2 = CausalEdge {
            from: "a".into(),
            to: "b".into(),
            edge_type: CausalEdgeType::Mitigates,
            strength: 0.9,
            confidence: nexus_cog_core::common::Confidence::new(1.0),
            evidence: vec![],
        };
        upsert_edge(b.as_ref(), &e1).unwrap();
        upsert_edge(b.as_ref(), &e2).unwrap();
        let edges = load_all_edges(b.as_ref()).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_type, CausalEdgeType::Mitigates);
    }
}
