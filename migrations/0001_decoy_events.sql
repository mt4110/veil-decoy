CREATE TABLE decoy_events (
    id TEXT PRIMARY KEY,
    token_id TEXT NOT NULL,
    observed_at INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'delivered', 'failed', 'rate_limited')),
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER NOT NULL,
    lease_id TEXT,
    lease_until INTEGER,
    last_error TEXT,
    payload TEXT
) STRICT;

CREATE INDEX decoy_events_by_token ON decoy_events(token_id, observed_at DESC, id DESC);
CREATE INDEX decoy_events_due ON decoy_events(next_attempt_at, id) WHERE state = 'pending';
CREATE INDEX decoy_events_retention ON decoy_events(observed_at);
