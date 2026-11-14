-- Append-only audit trail: every API request plus every domain event.
-- No foreign keys on purpose, so entries outlive the tickets they mention.
-- `credential` holds a token fingerprint (never the token itself).

CREATE TABLE audit_log (
    id          TEXT PRIMARY KEY NOT NULL,
    occurred_at TEXT NOT NULL,
    kind        TEXT NOT NULL,
    actor       TEXT,
    client_ip   TEXT,
    method      TEXT,
    path        TEXT,
    status      INTEGER,
    credential  TEXT,
    ticket_id   TEXT,
    summary     TEXT NOT NULL,
    detail      TEXT -- JSON
);

CREATE INDEX audit_log_occurred ON audit_log (occurred_at);
