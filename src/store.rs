use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct ClaimRow {
    pub id: i64,
    pub statement: String,
    pub status: String,
    pub rationale: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SourceRow {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub source_class: String,
    pub duplicate_of: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct EntityRow {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RelationshipRow {
    pub id: i64,
    pub from_entity_id: i64,
    pub from_name: String,
    pub to_entity_id: i64,
    pub to_name: String,
    pub relation: String,
    pub status: String,
    pub rationale: Option<String>,
}

#[derive(Clone, Debug)]
pub struct LeadRow {
    pub id: i64,
    pub description: String,
    pub status: String,
    pub rationale: Option<String>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("failed to open workspace {}", path.display()))?;
        conn.execute_batch(include_str!("../migrations/0001_init.sql"))
            .context("failed to initialize base workspace schema")?;
        conn.execute_batch(include_str!("../migrations/0002_epistemic_objects.sql"))
            .context("failed to initialize epistemic workspace schema")?;
        Ok(Self { conn })
    }

    pub fn create_case(&self, objective: &str) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO cases (id, objective, status, created_at) VALUES (?1, ?2, 'active', ?3)",
            params![id, objective, now()],
        )?;
        Ok(id)
    }

    pub fn finish_case(&self, case_id: &str, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE cases SET status = ?2, finished_at = ?3 WHERE id = ?1",
            params![case_id, status, now()],
        )?;
        Ok(())
    }

    pub fn record_event(&self, case_id: &str, kind: &str, message: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO events (case_id, kind, message, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![case_id, kind, message, now()],
        )?;
        Ok(())
    }

    pub fn upsert_source(
        &self,
        case_id: &str,
        url: &str,
        title: Option<&str>,
        source_class: &str,
        content: &str,
    ) -> Result<i64> {
        if let Some(id) = self
            .conn
            .query_row(
                "SELECT id FROM sources WHERE case_id = ?1 AND url = ?2",
                params![case_id, url],
                |row| row.get(0),
            )
            .optional()?
        {
            return Ok(id);
        }

        let content_hash = hash_text(content);
        let duplicate_of: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM sources WHERE case_id = ?1 AND content_hash = ?2 ORDER BY id LIMIT 1",
                params![case_id, content_hash],
                |row| row.get(0),
            )
            .optional()?;

        self.conn.execute(
            "INSERT INTO sources (case_id, url, title, source_class, content_hash, duplicate_of, retrieved_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                case_id,
                url,
                title,
                source_class,
                content_hash,
                duplicate_of,
                now()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn source_id_by_url(&self, case_id: &str, url: &str) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT id FROM sources WHERE case_id = ?1 AND url = ?2",
                params![case_id, url],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn record_claim(&self, case_id: &str, statement: &str) -> Result<i64> {
        require_text(statement, "claim statement")?;
        let timestamp = now();
        self.conn.execute(
            "INSERT INTO claims (case_id, statement, status, created_at, updated_at)
             VALUES (?1, ?2, 'UNRESOLVED', ?3, ?3)",
            params![case_id, statement, timestamp],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn record_evidence(
        &self,
        case_id: &str,
        source_id: i64,
        excerpt: &str,
        source_class: &str,
        directness: &str,
    ) -> Result<i64> {
        require_text(excerpt, "evidence excerpt")?;
        validate_source_class(source_class)?;
        if !matches!(directness, "DIRECT" | "INDIRECT") {
            bail!("directness must be DIRECT or INDIRECT");
        }

        let source_case: Option<String> = self
            .conn
            .query_row(
                "SELECT case_id FROM sources WHERE id = ?1",
                [source_id],
                |row| row.get(0),
            )
            .optional()?;
        if source_case.as_deref() != Some(case_id) {
            bail!("evidence must reference a scraped source in the current case");
        }

        self.conn.execute(
            "UPDATE sources SET source_class = ?2 WHERE id = ?1 AND source_class = 'unknown'",
            params![source_id, source_class],
        )?;
        self.conn.execute(
            "INSERT INTO evidence (case_id, source_id, excerpt, source_class, directness, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![case_id, source_id, excerpt, source_class, directness, now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn link_evidence(&self, claim_id: i64, evidence_id: i64, relation: &str) -> Result<()> {
        validate_evidence_relation(relation)?;
        ensure_claim_evidence_same_case(&self.conn, claim_id, evidence_id)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO claim_evidence (claim_id, evidence_id, relation) VALUES (?1, ?2, ?3)",
            params![claim_id, evidence_id, relation],
        )?;
        Ok(())
    }

    pub fn set_claim_status(
        &self,
        case_id: &str,
        claim_id: i64,
        status: &str,
        rationale: &str,
    ) -> Result<()> {
        validate_epistemic_status(status)?;
        let (supports, contradicts) = self.conn.query_row(
            "SELECT
                SUM(CASE WHEN ce.relation = 'SUPPORTS' THEN 1 ELSE 0 END),
                SUM(CASE WHEN ce.relation = 'CONTRADICTS' THEN 1 ELSE 0 END)
             FROM claim_evidence ce
             JOIN claims c ON c.id = ce.claim_id
             WHERE ce.claim_id = ?1 AND c.case_id = ?2",
            params![claim_id, case_id],
            evidence_counts,
        )?;
        enforce_epistemic_evidence(status, supports, contradicts)?;

        let updated = self.conn.execute(
            "UPDATE claims SET status = ?3, rationale = ?4, updated_at = ?5 WHERE id = ?1 AND case_id = ?2",
            params![claim_id, case_id, status, rationale, now()],
        )?;
        if updated == 0 {
            bail!("claim does not exist in the current case");
        }
        Ok(())
    }

    pub fn record_entity(
        &self,
        case_id: &str,
        name: &str,
        kind: &str,
        description: Option<&str>,
    ) -> Result<i64> {
        require_text(name, "entity name")?;
        require_text(kind, "entity kind")?;
        self.conn.execute(
            "INSERT INTO entities (case_id, name, kind, description, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![case_id, name, kind, description, now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn record_relationship(
        &self,
        case_id: &str,
        from_entity_id: i64,
        to_entity_id: i64,
        relation: &str,
    ) -> Result<i64> {
        require_text(relation, "relationship type")?;
        if from_entity_id == to_entity_id {
            bail!("a relationship must connect two distinct entity records");
        }
        ensure_entities_in_case(&self.conn, case_id, from_entity_id, to_entity_id)?;
        let timestamp = now();
        self.conn.execute(
            "INSERT INTO relationships
             (case_id, from_entity_id, to_entity_id, relation, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'UNRESOLVED', ?5, ?5)",
            params![case_id, from_entity_id, to_entity_id, relation, timestamp],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn link_relationship_evidence(
        &self,
        relationship_id: i64,
        evidence_id: i64,
        relation: &str,
    ) -> Result<()> {
        validate_evidence_relation(relation)?;
        ensure_relationship_evidence_same_case(&self.conn, relationship_id, evidence_id)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO relationship_evidence (relationship_id, evidence_id, relation)
             VALUES (?1, ?2, ?3)",
            params![relationship_id, evidence_id, relation],
        )?;
        Ok(())
    }

    pub fn set_relationship_status(
        &self,
        case_id: &str,
        relationship_id: i64,
        status: &str,
        rationale: &str,
    ) -> Result<()> {
        validate_epistemic_status(status)?;
        let (supports, contradicts) = self.conn.query_row(
            "SELECT
                SUM(CASE WHEN re.relation = 'SUPPORTS' THEN 1 ELSE 0 END),
                SUM(CASE WHEN re.relation = 'CONTRADICTS' THEN 1 ELSE 0 END)
             FROM relationship_evidence re
             JOIN relationships r ON r.id = re.relationship_id
             WHERE re.relationship_id = ?1 AND r.case_id = ?2",
            params![relationship_id, case_id],
            evidence_counts,
        )?;
        enforce_epistemic_evidence(status, supports, contradicts)?;

        let updated = self.conn.execute(
            "UPDATE relationships
             SET status = ?3, rationale = ?4, updated_at = ?5
             WHERE id = ?1 AND case_id = ?2",
            params![relationship_id, case_id, status, rationale, now()],
        )?;
        if updated == 0 {
            bail!("relationship does not exist in the current case");
        }
        Ok(())
    }

    pub fn record_lead(&self, case_id: &str, description: &str) -> Result<i64> {
        require_text(description, "lead description")?;
        let timestamp = now();
        self.conn.execute(
            "INSERT INTO leads (case_id, description, status, created_at, updated_at)
             VALUES (?1, ?2, 'OPEN', ?3, ?3)",
            params![case_id, description, timestamp],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn set_lead_status(
        &self,
        case_id: &str,
        lead_id: i64,
        status: &str,
        rationale: &str,
    ) -> Result<()> {
        validate_lead_status(status)?;
        let updated = self.conn.execute(
            "UPDATE leads SET status = ?3, rationale = ?4, updated_at = ?5
             WHERE id = ?1 AND case_id = ?2",
            params![lead_id, case_id, status, rationale, now()],
        )?;
        if updated == 0 {
            bail!("lead does not exist in the current case");
        }
        Ok(())
    }

    pub fn record_note(&self, case_id: &str, body: &str) -> Result<i64> {
        require_text(body, "note")?;
        self.conn.execute(
            "INSERT INTO notes (case_id, body, created_at) VALUES (?1, ?2, ?3)",
            params![case_id, body, now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_claims(&self, case_id: &str) -> Result<Vec<ClaimRow>> {
        let mut statement = self.conn.prepare(
            "SELECT id, statement, status, rationale FROM claims WHERE case_id = ?1 ORDER BY id",
        )?;
        let rows = statement.query_map([case_id], |row| {
            Ok(ClaimRow {
                id: row.get(0)?,
                statement: row.get(1)?,
                status: row.get(2)?,
                rationale: row.get(3)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_sources(&self, case_id: &str) -> Result<Vec<SourceRow>> {
        let mut statement = self.conn.prepare(
            "SELECT id, url, title, source_class, duplicate_of
             FROM sources WHERE case_id = ?1 ORDER BY id",
        )?;
        let rows = statement.query_map([case_id], |row| {
            Ok(SourceRow {
                id: row.get(0)?,
                url: row.get(1)?,
                title: row.get(2)?,
                source_class: row.get(3)?,
                duplicate_of: row.get(4)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_entities(&self, case_id: &str) -> Result<Vec<EntityRow>> {
        let mut statement = self.conn.prepare(
            "SELECT id, name, kind, description FROM entities WHERE case_id = ?1 ORDER BY id",
        )?;
        let rows = statement.query_map([case_id], |row| {
            Ok(EntityRow {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: row.get(2)?,
                description: row.get(3)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_relationships(&self, case_id: &str) -> Result<Vec<RelationshipRow>> {
        let mut statement = self.conn.prepare(
            "SELECT r.id, r.from_entity_id, f.name, r.to_entity_id, t.name,
                    r.relation, r.status, r.rationale
             FROM relationships r
             JOIN entities f ON f.id = r.from_entity_id
             JOIN entities t ON t.id = r.to_entity_id
             WHERE r.case_id = ?1
             ORDER BY r.id",
        )?;
        let rows = statement.query_map([case_id], |row| {
            Ok(RelationshipRow {
                id: row.get(0)?,
                from_entity_id: row.get(1)?,
                from_name: row.get(2)?,
                to_entity_id: row.get(3)?,
                to_name: row.get(4)?,
                relation: row.get(5)?,
                status: row.get(6)?,
                rationale: row.get(7)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_leads(&self, case_id: &str) -> Result<Vec<LeadRow>> {
        let mut statement = self.conn.prepare(
            "SELECT id, description, status, rationale FROM leads WHERE case_id = ?1 ORDER BY id",
        )?;
        let rows = statement.query_map([case_id], |row| {
            Ok(LeadRow {
                id: row.get(0)?,
                description: row.get(1)?,
                status: row.get(2)?,
                rationale: row.get(3)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn evidence_count_for_claim(&self, claim_id: i64) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM claim_evidence WHERE claim_id = ?1",
                [claim_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn evidence_count_for_relationship(&self, relationship_id: i64) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM relationship_evidence WHERE relationship_id = ?1",
                [relationship_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }
}

fn evidence_counts(row: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, i64)> {
    Ok((
        row.get::<_, Option<i64>>(0)?.unwrap_or(0),
        row.get::<_, Option<i64>>(1)?.unwrap_or(0),
    ))
}

fn enforce_epistemic_evidence(status: &str, supports: i64, contradicts: i64) -> Result<()> {
    match status {
        "VERIFIED" | "SUPPORTED" if supports == 0 => {
            bail!("{status} requires at least one supporting evidence record")
        }
        "DISPROVEN" if contradicts == 0 => {
            bail!("DISPROVEN requires at least one contradicting evidence record")
        }
        "CONFLICTING" if supports == 0 || contradicts == 0 => {
            bail!("CONFLICTING requires both supporting and contradicting evidence")
        }
        _ => Ok(()),
    }
}

fn ensure_claim_evidence_same_case(
    conn: &Connection,
    claim_id: i64,
    evidence_id: i64,
) -> Result<()> {
    let same_case: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM claims c JOIN evidence e ON e.case_id = c.case_id
             WHERE c.id = ?1 AND e.id = ?2",
            params![claim_id, evidence_id],
            |row| row.get(0),
        )
        .optional()?;
    if same_case.is_none() {
        bail!("claim and evidence must belong to the same case");
    }
    Ok(())
}

fn ensure_relationship_evidence_same_case(
    conn: &Connection,
    relationship_id: i64,
    evidence_id: i64,
) -> Result<()> {
    let same_case: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM relationships r JOIN evidence e ON e.case_id = r.case_id
             WHERE r.id = ?1 AND e.id = ?2",
            params![relationship_id, evidence_id],
            |row| row.get(0),
        )
        .optional()?;
    if same_case.is_none() {
        bail!("relationship and evidence must belong to the same case");
    }
    Ok(())
}

fn ensure_entities_in_case(
    conn: &Connection,
    case_id: &str,
    from_entity_id: i64,
    to_entity_id: i64,
) -> Result<()> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM entities WHERE case_id = ?1 AND id IN (?2, ?3)",
        params![case_id, from_entity_id, to_entity_id],
        |row| row.get(0),
    )?;
    if count != 2 {
        bail!("both relationship entities must belong to the current case");
    }
    Ok(())
}

fn validate_epistemic_status(status: &str) -> Result<()> {
    if matches!(
        status,
        "VERIFIED"
            | "SUPPORTED"
            | "UNRESOLVED"
            | "CONFLICTING"
            | "DISPROVEN"
            | "INSUFFICIENT_EVIDENCE"
            | "DEAD_END"
    ) {
        Ok(())
    } else {
        Err(anyhow!("invalid epistemic status: {status}"))
    }
}

fn validate_source_class(source_class: &str) -> Result<()> {
    if matches!(
        source_class,
        "PRIMARY" | "HIGH_QUALITY_SECONDARY" | "SECONDARY" | "WEAK"
    ) {
        Ok(())
    } else {
        Err(anyhow!("invalid source class: {source_class}"))
    }
}

fn validate_evidence_relation(relation: &str) -> Result<()> {
    if matches!(relation, "SUPPORTS" | "CONTRADICTS") {
        Ok(())
    } else {
        Err(anyhow!("evidence relation must be SUPPORTS or CONTRADICTS"))
    }
}

fn validate_lead_status(status: &str) -> Result<()> {
    if matches!(status, "OPEN" | "ACTIVE" | "EXHAUSTED" | "DISCARDED") {
        Ok(())
    } else {
        Err(anyhow!("invalid lead status: {status}"))
    }
}

fn require_text(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} cannot be empty");
    }
    Ok(())
}

fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_store() -> (tempfile::TempDir, Store, String) {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("workspace.db")).unwrap();
        let case = store.create_case("test").unwrap();
        (dir, store, case)
    }

    #[test]
    fn verified_claim_requires_supporting_evidence() {
        let (_dir, store, case) = test_store();
        let claim = store.record_claim(&case, "A is B").unwrap();

        assert!(
            store
                .set_claim_status(&case, claim, "VERIFIED", "no evidence")
                .is_err()
        );
    }

    #[test]
    fn conflicting_claim_requires_both_sides() {
        let (_dir, store, case) = test_store();
        let source = store
            .upsert_source(&case, "https://a.test", None, "unknown", "source")
            .unwrap();
        let claim = store.record_claim(&case, "A is B").unwrap();
        let evidence = store
            .record_evidence(&case, source, "supports", "PRIMARY", "DIRECT")
            .unwrap();
        store.link_evidence(claim, evidence, "SUPPORTS").unwrap();

        assert!(
            store
                .set_claim_status(&case, claim, "CONFLICTING", "one-sided")
                .is_err()
        );
    }

    #[test]
    fn verified_relationship_requires_supporting_evidence() {
        let (_dir, store, case) = test_store();
        let a = store.record_entity(&case, "A", "company", None).unwrap();
        let b = store.record_entity(&case, "B", "company", None).unwrap();
        let relationship = store.record_relationship(&case, a, b, "controls").unwrap();

        assert!(
            store
                .set_relationship_status(&case, relationship, "VERIFIED", "no evidence")
                .is_err()
        );
    }

    #[test]
    fn duplicate_content_is_linked() {
        let (_dir, store, case) = test_store();
        let first = store
            .upsert_source(&case, "https://a.test", None, "PRIMARY", "same")
            .unwrap();
        let second = store
            .upsert_source(&case, "https://b.test", None, "SECONDARY", "same")
            .unwrap();
        let sources = store.list_sources(&case).unwrap();

        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].id, first);
        assert_eq!(sources[1].id, second);
        assert_eq!(sources[1].duplicate_of, Some(first));
    }

    #[test]
    fn lead_state_is_validated() {
        let (_dir, store, case) = test_store();
        let lead = store.record_lead(&case, "Find ownership filing").unwrap();

        assert!(
            store
                .set_lead_status(&case, lead, "ACTIVE", "working")
                .is_ok()
        );
        assert!(
            store
                .set_lead_status(&case, lead, "UNKNOWN", "bad state")
                .is_err()
        );
    }
}
