PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS cases (
    id TEXT PRIMARY KEY,
    objective TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL,
    finished_at TEXT
);

CREATE TABLE IF NOT EXISTS sources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id TEXT NOT NULL,
    url TEXT NOT NULL,
    title TEXT,
    source_class TEXT NOT NULL DEFAULT 'unknown',
    content_hash TEXT NOT NULL,
    duplicate_of INTEGER,
    retrieved_at TEXT NOT NULL,
    UNIQUE(case_id, url),
    FOREIGN KEY(case_id) REFERENCES cases(id) ON DELETE CASCADE,
    FOREIGN KEY(duplicate_of) REFERENCES sources(id)
);

CREATE INDEX IF NOT EXISTS idx_sources_case ON sources(case_id);
CREATE INDEX IF NOT EXISTS idx_sources_hash ON sources(case_id, content_hash);

CREATE TABLE IF NOT EXISTS claims (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id TEXT NOT NULL,
    statement TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'UNRESOLVED',
    rationale TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(case_id) REFERENCES cases(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_claims_case ON claims(case_id);

CREATE TABLE IF NOT EXISTS evidence (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    excerpt TEXT NOT NULL,
    source_class TEXT NOT NULL,
    directness TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(case_id) REFERENCES cases(id) ON DELETE CASCADE,
    FOREIGN KEY(source_id) REFERENCES sources(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_evidence_case ON evidence(case_id);

CREATE TABLE IF NOT EXISTS claim_evidence (
    claim_id INTEGER NOT NULL,
    evidence_id INTEGER NOT NULL,
    relation TEXT NOT NULL,
    PRIMARY KEY(claim_id, evidence_id, relation),
    FOREIGN KEY(claim_id) REFERENCES claims(id) ON DELETE CASCADE,
    FOREIGN KEY(evidence_id) REFERENCES evidence(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(case_id) REFERENCES cases(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_events_case ON events(case_id, id);
