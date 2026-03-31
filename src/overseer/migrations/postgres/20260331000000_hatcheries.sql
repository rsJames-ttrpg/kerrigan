CREATE TABLE hatcheries (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    status TEXT NOT NULL DEFAULT 'online' CHECK (status IN ('online', 'degraded', 'offline')),
    capabilities JSONB NOT NULL DEFAULT '{}',
    max_concurrency INTEGER NOT NULL DEFAULT 1,
    active_drones INTEGER NOT NULL DEFAULT 0,
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE job_runs ADD COLUMN hatchery_id TEXT REFERENCES hatcheries(id);
