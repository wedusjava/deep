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
            .context("failed to initialize workspace schema")?;
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
        if excerpt.trim().is_empty() {
            bail!("evidence excerpt cannot be empty");
        }
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
        if !matches!(relation, "SUPPORTS" | "CONTRADICTS") {
            bail!("relation must be SUPPORTS or CONTRADICTS");
        }

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
        validate_claim_status(status)?;

        let (supports, contradicts): (i64, i64) = self.conn.query_row(
            "SELECT
                SUM(CASE WHEN ce.relation = 'SUPPORTS' THEN 1 ELSE 0 END),
                SUM(CASE WHEN ce.relation = 'CONTRADICTS' THEN 1 ELSE 0 END)
             FROM claim_evidence ce
             JOIN claims c ON c.id = ce.claim_id
             WHERE ce.claim_id = ?1 AND c.case_id = ?2",
            params![claim_id, case_id],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?.unwrap_or(0),
                    row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                ))
            },
        )?;

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
            _ => {}
        }

        let updated = self.conn.execute(
            "UPDATE claims SET status = ?3, rationale = ?4, updated_at = ?5 WHERE id = ?1 AND case_id = ?2",
            params![claim_id, case_id, status, rationale, now()],
        )?;
        if updated == 0 {
            bail!("claim does not exist in the current case");
        }
        Ok(())
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
            "SELECT id, url, title, source_class, duplicate_of FROM sources WHERE case_id = ?1 ORDER BY id",
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

    pub fn evidence_count_for_claim(&self, claim_id: i64) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM claim_evidence WHERE claim_id = ?1",
                [claim_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }
}

fn validate_claim_status(status: &str) -> Result<()> {
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
        Err(anyhow!("invalid claim status: {status}"))
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

    #[test]
    fn verified_claim_requires_supporting_evidence() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("workspace.db")).unwrap();
        let case = store.create_case("test").unwrap();
        let claim = store.record_claim(&case, "A is B").unwrap();

        assert!(
            store
                .set_claim_status(&case, claim, "VERIFIED", "no evidence")
                .is_err()
        );
    }

    #[test]
    fn conflicting_claim_requires_both_sides() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("workspace.db")).unwrap();
        let case = store.create_case("test").unwrap();
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
    fn duplicate_content_is_linked() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("workspace.db")).unwrap();
        let case = store.create_case("test").unwrap();
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
}
