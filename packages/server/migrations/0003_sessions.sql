-- Verified play sessions. A die joins by NFC tap or BLE; rolls made while
-- joined carry the session id inside the signed record.
CREATE TABLE sessions (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Short human-typable code, also written to the NFC payload.
    join_code    TEXT NOT NULL UNIQUE,
    name         TEXT NOT NULL,
    organizer_id UUID REFERENCES users (id) ON DELETE SET NULL,
    -- Organizer lock: dice cannot change die type/count while locked.
    locked       BOOLEAN NOT NULL DEFAULT false,
    -- Allowed dice, e.g. {"die_sides": 20, "count": 1}.
    die_config   JSONB,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at     TIMESTAMPTZ
);

CREATE TABLE session_devices (
    session_id UUID NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    device_id  UUID NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    joined_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (session_id, device_id)
);
