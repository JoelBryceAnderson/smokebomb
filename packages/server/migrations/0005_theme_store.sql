-- Theme store: pre-rendered smoke styles and animation packs.
CREATE TABLE themes (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug         TEXT NOT NULL UNIQUE,
    name         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    kind         TEXT NOT NULL CHECK (kind IN ('smoke_style', 'animation_pack')),
    price_cents  INTEGER NOT NULL DEFAULT 0 CHECK (price_cents >= 0),
    -- Asset pack in the SMKB format (see smokebomb_shared::assets).
    asset_url    TEXT NOT NULL,
    asset_sha256 BYTEA NOT NULL CHECK (octet_length(asset_sha256) = 32),
    asset_size   BIGINT NOT NULL CHECK (asset_size BETWEEN 1 AND 67108864),
    min_firmware TEXT NOT NULL DEFAULT '0.1.0',
    published    BOOLEAN NOT NULL DEFAULT false,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE theme_purchases (
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    theme_id     UUID NOT NULL REFERENCES themes (id) ON DELETE RESTRICT,
    purchased_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, theme_id)
);
