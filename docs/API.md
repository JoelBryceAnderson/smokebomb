# Sugarcube REST API

Base URL: `http://127.0.0.1:8080` in development. All bodies are JSON, and
versioned endpoints live under `/v1`. Byte fields (serials, keys, digests,
signatures) are **lowercase hex**.

Status legend: ✅ implemented · 🧪 stub (fixed response or `501`) · 🗓 planned

## Errors

Every error has the same shape:

```json
{ "error": "bad_request", "message": "public_key must be 64 bytes" }
```

| HTTP | `error` | When |
|---|---|---|
| 400 | `bad_request` | Malformed hex, wrong byte length, unknown die, value out of range |
| 401 | `unauthorized` | Missing `Authorization: Bearer …` on an authenticated route |
| 404 | `not_found` | Unknown device, etc. |
| 501 | `not_implemented` | Stubbed endpoint |
| 500 | `internal` | Database or other server error (details are logged, not returned) |

## Authentication

Accounts get a short-lived HS256 JWT from `/v1/auth/login` and send it as
`Authorization: Bearer <token>`. Devices never use JWTs: a die proves who it is
by signing with its ATECC608 key, and the phone relays those signatures.

## Health

### `GET /health` ✅

```json
{ "status": "ok", "version": "0.1.0" }
```

### `GET /health/db` ✅

Runs `SELECT 1`. Returns `{ "status": "ok" }`, or `500`.

## Auth

### `POST /v1/auth/register` 🧪

```json
{ "email": "ada@example.com", "password": "…", "display_name": "Ada" }
```

Returns `501` for now. Planned: `201` with the `User` object.

### `POST /v1/auth/login` 🧪

```json
{ "email": "ada@example.com", "password": "…" }
```

Returns `501` for now. Planned: `{ "access_token": "…", "refresh_token": "…", "expires_in": 900 }`.

### `GET /v1/auth/me` 🧪

Needs a bearer token. Returns `401` without one and `501` with one.

## Devices

A device is identified by its 9-byte ATECC608 serial (18 hex chars).

### `POST /v1/devices` ✅

Registers a die. If the serial already exists, it updates the firmware
version and `last_seen_at` instead.

```json
{
  "serial": "01235b0e00000000ee",
  "public_key": "d62cf27d…b394",
  "firmware_version": "0.1.0"
}
```

| Field | Format |
|---|---|
| `serial` | 9 bytes, hex |
| `public_key` | 64 bytes, raw uncompressed P-256 point `X ‖ Y` (no `04` prefix), hex |
| `firmware_version` | semver string |

`201 Created`:

```json
{
  "id": "5b7a…",
  "serial": "01235b0e00000000ee",
  "public_key": "d62cf27d…b394",
  "owner_name": null,
  "firmware_version": "0.1.0",
  "registered_at": "2026-09-28T01:52:10.123Z",
  "last_seen_at": "2026-09-28T01:52:10.123Z"
}
```

> ⚠️ Registration isn't authenticated yet. Before launch it must require a
> factory attestation, meaning the device key signed by the Sugarcube
> manufacturing CA.

### `GET /v1/devices/{serial}` ✅

Returns the same object as above, or `404`.

### `GET /v1/devices/{serial}/rolls` ✅

The device's last 100 stored rolls, newest first. The list stays empty until
roll upload is implemented.

```json
[
  { "counter": 12, "die": "d20", "values": [17], "digest": "…", "received_at": "…" }
]
```

## Rolls

A signed roll is the hex/JSON form of `smokebomb_shared::SignedRoll`:

| Field | Type | Notes |
|---|---|---|
| `device_serial` | hex, 9 B | |
| `session` | hex, 16 B, optional | Omit or send zeros for a casual roll |
| `counter` | u32 | ATECC608 monotonic counter |
| `uptime_ms` | u64 | Device uptime at roll time |
| `die` | `d4`, `d6`, `d8`, `d10`, `d12`, `d20`, `d100` or `pass_the_pot` | |
| `values` | 1–10 raw values (1–3 for `pass_the_pot`), each in `1..=sides` | Pass the Pot is signed as d6 values: 1 → ←, 2 → P, 3 → →, 4–6 → • |
| `prev_hash` | hex, 32 B | Digest of the previous roll; all zeros for the first |
| `signature` | hex, 64 B | ECDSA P-256 `r ‖ s` over `digest` |

`digest = SHA-256(encode(record))`, where `encode` is the fixed layout in
[ARCHITECTURE.md](ARCHITECTURE.md#rolls-and-verification). The server always
recomputes the digest itself and never accepts one from the client.

### `POST /v1/rolls/verify` ✅

Checks the signature against the registered device key and checks the
hash-chain link to the stored previous roll. Nothing is stored.

Request: a signed roll (above).

```json
{
  "device_serial": "01235b0e00000000ee",
  "counter": 0,
  "uptime_ms": 20296,
  "die": "d6",
  "values": [3],
  "prev_hash": "0000000000000000000000000000000000000000000000000000000000000000",
  "signature": "1436…"
}
```

`200 OK`:

```json
{ "valid": true, "digest": "671b…761f", "chain": "genesis", "reason": null }
```

| `chain` | Meaning |
|---|---|
| `genesis` | `counter == 0` and `prev_hash` is all zeros |
| `linked` | `prev_hash` equals the stored digest at `counter - 1` |
| `unknown` | The previous roll isn't on file yet, so the link can't be checked |
| `broken` | The stored previous digest differs. `valid` is `false` |

`valid` is `true` when the signature verifies and the chain isn't `broken`.
`reason` explains a signature failure (`"signature does not match"`, …).
The request fails with `400` if the device isn't registered, a field is
malformed, the die is unknown, or the number of dice or a value is out of
range for the die.

The die is part of the signed record, so the same values claimed as a
different die (for example Pass the Pot sent as `d6`) fail verification.

### `POST /v1/rolls` 🧪

Uploads a signed roll for history sync; the phone relays these from the die.
Returns `501` for now. Planned behaviour: verify as above, store the roll in
`rolls` (idempotent on `(device, counter)`), and return the stored roll.

## Themes

### `GET /v1/themes` ✅

Lists published themes, newest first.

```json
[
  {
    "id": "…",
    "slug": "ember",
    "name": "Ember",
    "description": "Warm sparks that burst on a max roll.",
    "kind": "smoke_style",
    "price_cents": 299,
    "asset_url": "https://cdn…/ember.smkb",
    "asset_size": 18874368,
    "min_firmware": "0.1.0",
    "created_at": "…"
  }
]
```

`kind` is `smoke_style` or `animation_pack`. `asset_url` points to an SMKB
pack of at most 64 MB. The phone downloads it, checks `asset_sha256` (not in
the response yet) and streams it to the die over BLE.

## Planned

| Endpoint | Purpose |
|---|---|
| `POST /v1/auth/refresh` | Rotate the refresh token |
| `PATCH /v1/devices/{serial}` | Set the owner and owner name (owner only) |
| `POST /v1/sessions` / `GET /v1/sessions/{code}` | Create or look up a verified session (organizer) |
| `POST /v1/sessions/{id}/lock` | Organizer lock |
| `GET /v1/sessions/{id}/rolls` | Live roll feed for a table |
| `POST /v1/themes/{slug}/purchase` | Purchase and get a download entitlement |
| `GET /verify/{digest}` | Public web page that shows whether a roll is authentic |
