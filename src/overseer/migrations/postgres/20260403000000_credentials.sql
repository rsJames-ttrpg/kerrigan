CREATE TABLE credentials (
    id TEXT PRIMARY KEY,
    pattern TEXT NOT NULL,
    credential_type TEXT NOT NULL,
    secret TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(pattern, credential_type)
);
