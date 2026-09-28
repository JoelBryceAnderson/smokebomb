-- Player / organizer accounts.
CREATE TABLE users (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email         TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    -- argon2id PHC string; NULL for accounts created via a future OAuth flow.
    password_hash TEXT,
    role          TEXT NOT NULL DEFAULT 'player'
                  CHECK (role IN ('player', 'organizer', 'admin')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX users_email_key ON users (lower(email));
