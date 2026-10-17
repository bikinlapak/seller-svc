# 03 — API surface (seller-svc / BFF)

Every endpoint is screen-shaped: one request per screen where possible, so the
PWA on a mid-range Android over 3G makes one round trip, not six. Responses are
already joined and formatted (Rupiah as integer minor units, dates in WIB).

Prefix: `/v1`. Health is unversioned. All error bodies are
`{ "error": { "code", "message" } }` — the existing `AppError` contract.

## Route table

```plain text
Health (no auth)
  GET    /healthz                          liveness, does not touch works-server
  GET    /readyz                           works-server + Postgres + Redis reachable

Auth & onboarding                          routes/auth.rs, routes/onboarding.rs
  POST   /v1/auth/otp/start                WhatsApp OTP, rate limited per msisdn
  POST   /v1/auth/otp/verify               → session cookie + works.io tokens
  POST   /v1/auth/google                   Google sign-in
  POST   /v1/auth/refresh                  rotate the works.io access token
  DELETE /v1/auth/session                  logout
  POST   /v1/onboarding/shop               name + slug availability, address
  POST   /v1/onboarding/template           storefront template pick
  POST   /v1/onboarding/publish            pre-flight checks, then publish
  GET    /v1/onboarding/checklist          "Lapakmu 4/6 siap"

Catalog & stock                            routes/products.rs, routes/stock.rs
  GET    /v1/products                      search, category, stock band, cursor
  POST   /v1/products
  GET    /v1/products/:id
  PATCH  /v1/products/:id
  POST   /v1/products/:id/images           V4 signed PUT URL
  POST   /v1/products/suggest              AI listing from a photo (labelled AI)
  POST   /v1/products/import               CSV preview + commit
  GET    /v1/stock?location=
  GET    /v1/lots?nearExpiryDays=
  POST   /v1/stock/adjust                  reason picker, optional lotId
  POST   /v1/stock/opname                  count by aisle, submit, post

Suppliers & purchasing                     routes/suppliers.rs, purchase_orders.rs
  GET    /v1/suppliers
  POST   /v1/suppliers
  PATCH  /v1/suppliers/:id                 lead time, pack size, MOQ
  GET    /v1/purchase-orders
  POST   /v1/purchase-orders
  GET    /v1/purchase-orders/:id
  POST   /v1/purchase-orders/:id/receive   lot + expiry entry on receipt

Orders & reports                           routes/orders.rs, routes/reports.rs
  GET    /v1/orders                        lapak, marketplace, chat-to-checkout
  GET    /v1/orders/:id
  POST   /v1/orders/:id/fulfil             process, ship, tracking number
  GET    /v1/home                          assistant-home aggregate (one request)
  GET    /v1/reports/summary               revenue, profit, best sellers, stock value
  GET    /v1/reports/waste?month=
  GET    /v1/reports/monthly?month=        PDF link + badge state

Shop Assistant                               routes/assistant.rs, routes/actions.rs
  GET    /v1/assistant/thread              history, cursor-paged
  GET    /v1/assistant/stream              SSE
  POST   /v1/assistant/messages            merchant message → agent
  POST   /v1/assistant/read                read receipts
  GET    /v1/assistant/settings            watched SKUs, channels, auto-approve
  PUT    /v1/assistant/settings
  POST   /v1/assistant/activate            Aktifkan + Kesiapan answers
  POST   /v1/assistant/report              "Laporkan jawaban" → moderation queue
  POST   /v1/actions/:token                forwarded to passtok-executor

Waste                                      routes/waste.rs, recipients.rs, donations.rs
  POST   /v1/waste/photos                  V4 signed PUT URL, per-tenant prefix
  POST   /v1/waste/analyze                 → agent vision, grounded candidates
  POST   /v1/waste/confirm                 → executor adjustStock
  GET    /v1/waste/destinations            options filtered by expiry state
  GET    /v1/recipients
  POST   /v1/recipients
  POST   /v1/donations                     checklist gate, pickup window
  GET    /v1/donations/:id/receipt.pdf
  POST   /v1/specials                      daily special approve → work order

Notifications                              routes/push.rs
  POST   /v1/push/subscribe
  DELETE /v1/push/subscribe
```

## Conventions

**Auth.** An `HttpOnly`, `Secure`, `SameSite=Lax` session cookie holds an opaque
session id; the worksh.io access and refresh tokens are stored server-side in
Redis keyed by that id, never sent to the browser. Every works-server call is
made with the merchant's own token, so works-server enforces the tenant and the
permissions — `seller-svc` does not re-implement authorization. A 401 from
works-server surfaces as `AppError::Unauthorized`; the BFF tries one silent
refresh before returning it.

**Mutations never go direct.** Anything that changes worksh.io state from a
suggestion goes through `POST /v1/actions/:token` → `passtok-executor`. Plain
CRUD the merchant typed themselves (edit a product, adjust stock by hand) goes
straight to works-server from the BFF — no token needed, because the merchant's
intent is the request itself. The dividing line: _did an AI propose this?_

**Idempotency.** Every POST that creates something accepts an
`Idempotency-Key` header; the BFF stores the key → response for 24h in Redis.
Required for the action and confirm endpoints, where a flaky mobile connection
retrying is normal.

**SSE.** `GET /v1/assistant/stream` emits named events — `message`,
`suggestion`, `result`, `resync`, `ping` (every 25s, to survive proxy idle
timeouts) — each with a monotonic `id` so the browser's `Last-Event-ID` can
resume. On lag, send `resync` and let the client refetch the thread rather than
guessing what it missed.

**Pagination.** Cursor-based (opaque, base64 of the sort key + id). No offsets:
the lists that matter (movements, messages) are append-heavy.

**Rate limits.** Redis token bucket per session and per IP on OTP start, message
send, analyze and suggest — the four endpoints that cost money or send SMS.

**Errors.** `AppError` keeps mapping to a stable `code`; add variants rather
than reusing `Internal` for anything a client should branch on
(`TOKEN_USED`, `TOKEN_EXPIRED`, `QTY_ABOVE_CAP`, `VISION_LOW_CONFIDENCE`,
`WORKS_PERMISSION_DENIED`, `RATE_LIMITED`). Internal and upstream detail stays
in the log, as it does now.

## Internal endpoints (not reachable from the browser)

Cloud Run IAM, internal ingress only, caller authenticated by its service
account:

```plain text
passtok-executor
  POST   /internal/execute                 token + final payload → mutation
passtok-agent
  POST   /internal/events                  Pub/Sub push (OIDC verified)
  POST   /internal/chat                    BFF forwards a merchant message
  POST   /internal/vision                  BFF forwards a waste photo
seller-svc
  POST   /internal/messages                agent posts a card into a thread
passtok-notifier
  POST   /internal/events                  Pub/Sub push (OIDC verified)
passtok-risk
  POST   /internal/events                  Pub/Sub push (OIDC verified)
  POST   /internal/run                     Scheduler: nightly rule pass
```

The agent posting back through `seller-svc` rather than writing `messages`
itself is deliberate: message persistence and SSE fan-out stay in one place, and
the agent's database credentials stay read-only.
