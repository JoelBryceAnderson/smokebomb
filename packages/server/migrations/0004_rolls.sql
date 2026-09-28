-- Signed roll records. Each row is one `smokebomb_shared::RollRecord` plus
-- its ATECC608 signature; `prev_hash` links to the device's previous roll.
CREATE TABLE rolls (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    device_id        UUID NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    session_id       UUID REFERENCES sessions (id) ON DELETE SET NULL,
    -- Secure element monotonic counter.
    counter          BIGINT NOT NULL CHECK (counter >= 0),
    die_sides        SMALLINT NOT NULL CHECK (die_sides IN (4, 6, 8, 10, 12, 20, 100)),
    dice             SMALLINT[] NOT NULL CHECK (cardinality(dice) BETWEEN 1 AND 6),
    digest           BYTEA NOT NULL CHECK (octet_length(digest) = 32),
    prev_hash        BYTEA NOT NULL CHECK (octet_length(prev_hash) = 32),
    signature        BYTEA NOT NULL CHECK (octet_length(signature) = 64),
    device_uptime_ms BIGINT NOT NULL,
    received_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (device_id, counter)
);

CREATE INDEX rolls_session_idx ON rolls (session_id) WHERE session_id IS NOT NULL;
CREATE UNIQUE INDEX rolls_digest_key ON rolls (digest);
