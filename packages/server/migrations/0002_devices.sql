-- Device registry: one row per physical die.
CREATE TABLE devices (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- ATECC608 serial number, lowercase hex (9 bytes).
    serial           TEXT NOT NULL UNIQUE CHECK (serial ~ '^[0-9a-f]{18}$'),
    -- Raw P-256 public key X || Y from the secure element.
    public_key       BYTEA NOT NULL CHECK (octet_length(public_key) = 64),
    owner_id         UUID REFERENCES users (id) ON DELETE SET NULL,
    -- Name shown on the die, set during onboarding.
    owner_name       TEXT,
    firmware_version TEXT NOT NULL,
    registered_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX devices_owner_idx ON devices (owner_id);
