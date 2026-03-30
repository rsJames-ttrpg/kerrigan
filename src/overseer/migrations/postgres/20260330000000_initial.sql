CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE memories (
    id TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    source TEXT NOT NULL,
    tags JSONB NOT NULL DEFAULT '[]',
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE memory_links (
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    linked_id TEXT NOT NULL,
    linked_type TEXT NOT NULL CHECK (linked_type IN ('memory', 'decision')),
    relation_type TEXT NOT NULL,
    PRIMARY KEY (memory_id, linked_id)
);

CREATE TABLE memory_embeddings (
    id BIGSERIAL PRIMARY KEY,
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    embedding vector,
    UNIQUE(memory_id, provider)
);

CREATE INDEX idx_memory_embeddings_provider ON memory_embeddings (provider);

CREATE TABLE job_definitions (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    config JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE job_runs (
    id TEXT PRIMARY KEY,
    definition_id TEXT NOT NULL REFERENCES job_definitions(id),
    parent_id TEXT REFERENCES job_runs(id),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    triggered_by TEXT NOT NULL,
    result JSONB,
    error TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ
);

CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    run_id TEXT REFERENCES job_runs(id),
    subject TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'in_progress', 'completed', 'failed')),
    assigned_to TEXT,
    output JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE decisions (
    id TEXT PRIMARY KEY,
    agent TEXT NOT NULL,
    context TEXT NOT NULL,
    decision TEXT NOT NULL,
    reasoning TEXT NOT NULL DEFAULT '',
    tags JSONB NOT NULL DEFAULT '[]',
    run_id TEXT REFERENCES job_runs(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE artifacts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size BIGINT NOT NULL,
    run_id TEXT REFERENCES job_runs(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
