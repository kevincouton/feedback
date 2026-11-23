# feedback-server

Self-hosted feedback service for the [lucanian.app](https://lucanian.app) product
fleet. Rust (Axum + sqlx/SQLite), single binary, no external services.

## Endpoints (public, behind Caddy at `https://feedback.lucanian.app`)

| Route | Description |
| --- | --- |
| `POST /api/feedback` | Submit feedback. Body: `{product, page, category, message, email?, website?}` where `category` is `useful` \| `bug` \| `feature`. `website` is a honeypot: if non-empty, the request is silently dropped (fake success, nothing stored). Returns `201 {id, status_url}`. |
| `GET /f/{id}` | Public, server-rendered status page (no login). |
| `GET /api/feedback/{id}/status` | Same status as JSON, for the widget. |
| `GET /widget.js` | Dependency-free embeddable feedback widget. |
| `GET /healthz` | `200 text/plain`. |

## Admin endpoints (localhost only, `127.0.0.1:31013`, used over SSH)

| Route | Description |
| --- | --- |
| `GET /admin/queue?product=&status=` | Triage queue (newest first, max 500). |
| `POST /admin/feedback/{id}/status` | Set status: `received` \| `in_review` \| `planned` \| `resolved`. |

## Email policy

`email` is **optional** and never required. It is stored solely so the team can
reply about that specific submission. It is never shared, never used for
marketing, and there is no mailing-list, newsletter, or tracking code anywhere
in this service — by policy and by schema. Automated triage is metadata-only:
raw feedback, email addresses, page URLs, and query strings are never exported
to GitHub issues or repository reports. Human review stays behind the
localhost-only admin queue; any reply requires explicit approval.

## Rate limiting

Per-IP token bucket: 5 submissions / 10 min / IP on `POST /api/feedback`;
`429` + `Retry-After` on overflow. In-memory, resets on restart.

## Widget integration

Add one line to the product's `app.html` (`<head>`):

```html
<script src="https://feedback.lucanian.app/widget.js" data-product="gittrend" defer></script>
```

`data-product` is required; `data-position="bottom-left|top-right|top-left"`
is optional (default `bottom-right`). The widget sets no cookies, does no
fingerprinting, and its only network call is the submission POST.

## Configuration

`/etc/feedback/env` (chmod 600, systemd `EnvironmentFile`):

```
API_PORT=31012
ADMIN_PORT=31013
DATABASE_URL=file:/var/lib/feedback/feedback.db
# SENTRY_DSN=            # optional, unset initially
# CORS_ORIGINS=          # optional comma-separated override; defaults to the
                         # deployed product origins
```

SQLite runs in WAL mode with a 5s busy timeout. Migrations are embedded via
`sqlx::migrate!` and run at startup.

## Development

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
```

Style: [RUST_STYLE.md](RUST_STYLE.md) (Google Rust Style Guide house standard).

## Privacy model

Status links (`/f/{id}`) are the only access control: whoever holds the
unguessable 128-bit id can view that submission's status. No accounts, no
identity checks beyond the token.
