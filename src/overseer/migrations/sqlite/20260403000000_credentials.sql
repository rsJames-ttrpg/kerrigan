CREATE TABLE credentials (
    id TEXT PRIMARY KEY,
    pattern TEXT NOT NULL,
    credential_type TEXT NOT NULL,
    secret TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(pattern, credential_type)
);
