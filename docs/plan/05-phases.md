# 05 — Phased plan (backend)

Phase numbers match the Notion build plan and the frontend plan, so a phase is a
joint milestone: `seller-app` Phase 2 and `seller-svc` Phase 2 ship together.

**Hackathon cut line:** Phases 0–8 are the demo build (every P0 in the PRD).
Phase 9 is demo hardening. Phases 10–12 are after the hackathon. The deadline is
not recorded yet, so phases are ordered by dependency, each sized S (≤1 day),
M (2–3 days) or L (4–5 days) for one engineer.

Build the **demo path first, breadth second**: within any phase, the tasks that
the 7-minute demo touches come before the ones that only make the product
complete.

> **Progress, 2026-10-15.** The HTTP layer `seller-app` consumes is built and
> under test (32 tests): contract types, the seven endpoints, error mapping,
> and the full action-token path with Ed25519 signing, single use, tenant and
> quantity-cap checks. `seller-app` runs against it with `SELLER_SVC_MOCK=false`
> and the approve → PO path works end to end in a browser.
>
> What is **not** real yet: the data. `src/seed.rs` stands in for worksh.io and
> the `passtok` schema, so every endpoint serves the right shape with demo
> content. No GraphQL call, no Postgres, no BigQuery, no Pub/Sub, no Gemini.
> Items are ticked only where they are genuinely done.

---

## Phase 0 — Foundation (M)

**Exit:** `seller-svc` deployed on Cloud Run, logging in against worksh.io
staging, CI green, Terraform applied.

- [ ] GCP project `passtok`: billing, budget and alerts (Rp cap per month)
- [ ] Terraform in `infra/`: Artifact Registry, Cloud Run services + the
      extractor job, Pub/Sub topics + DLQs, BigQuery datasets, GCS buckets
      (photos, receipts) with lifecycle rules, Memorystore Redis, Secret
      Manager, one service account per service with least privilege
- [ ] Enable Vertex AI; pick Gemini model versions and region
      (`asia-southeast2`, falling back to `asia-southeast1`) → **ADR-0006**
- [ ] Convert `seller-svc` to a Cargo workspace; `git mv src → crates/seller-svc/src`;
      create `passtok-core`, `passtok-worksh` and the five service crates as
      stubs that build and serve `/healthz`
- [x] `src/contract.rs` — the response types `seller-app` consumes, camelCase
- [x] `src/token.rs` — action tokens: Ed25519 JWT, single use, tenant and
      quantity-cap enforcement, concurrency-tested
- [x] `src/seed.rs` — Toko Kopi Jasa stand-in until worksh.io and `passtok` land
- [x] Config: `SELLER_SVC_ACTION_KEY` (PKCS#8 DER, base64); an ephemeral key is
      generated when unset, with a loud warning
- [x] Error codes a client can branch on: `token_invalid`, `token_expired`,
      `token_used`, `qty_above_cap`
- [ ] `passtok-core`: OTel tracing, event envelope + JSON Schema validation,
      money as an `Idr` newtype, ULID ids as a shared crate. The workspace
      split waits until a second deploy unit actually exists
- [ ] `passtok-worksh`: vendor the works-server SDL, `cynic` codegen, port the
      existing `WorksClient` behind a typed facade. **Blocked:** needs access
      to the works-server schema
- [ ] Session auth: cookie + Redis-backed worksh.io token store, silent refresh,
      `/readyz`
- [ ] CI (GitHub Actions or Cloud Build): fmt, clippy `-D warnings`, test,
      `cargo deny`, the `aws-lc-sys` check, build, deploy to staging on main;
      per-PR preview deploy
- [ ] ADRs 0001–0006 ([01-architecture.md](01-architecture.md) lists them)

## Phase 1 — Daftar & onboarding (M)

**Exit:** a new seller registers, finishes the wizard, and has a live lapak and
a tenant in worksh.io.

- [ ] `POST /v1/auth/otp/start|verify` — WhatsApp OTP, Redis rate limit per
      msisdn, resend backoff, terms acceptance recorded
- [ ] `POST /v1/auth/google`
- [ ] Tenant provisioning via worksh.io (`verifyRegistrationCode`, account
      onboarding) with `business_type` and `acquisition_brand = bikinlapak`
- [ ] Wizard endpoints: shop name + slug availability, template pick, first
      product, payment method (manual transfer + COD; Midtrans later), shipping
      origin + couriers, seller identity
- [ ] `POST /v1/onboarding/publish` with pre-flight checks; return URL + QR
- [ ] `GET /v1/onboarding/checklist` → "Lapakmu 4/6 siap"
- [ ] Funnel events: `seller.signup_completed`, `onboarding.step_completed`,
      `shop.published`

## Phase 2 — Produk & stok (L)

**Exit:** a merchant manages catalog, stock, lots, suppliers and POs from the app.

- [ ] Products: list (search, category, stock band, cursor), detail, create,
      edit; image upload via V4 signed URL
- [ ] `product_extras` + `supplier_terms` migrations and CRUD (perishable, shelf
      life, track lots, lead time, pack size, MOQ) — the works-core stand-ins
- [ ] `POST /v1/products/suggest` — Gemini proposes title, category, attributes,
      description from a photo; response carries `ai_label`; merchant approves
- [ ] `POST /v1/products/import` — CSV preview with per-row errors, then commit
- [ ] `GET /v1/stock`, `GET /v1/lots?nearExpiryDays=`, `POST /v1/stock/adjust`
      with reason codes
- [ ] `POST /v1/stock/opname` — count by aisle, submit, post
- [ ] Suppliers CRUD; purchase orders: list, detail, create, submit, receive
      (lot + expiry entry on receipt)
- [ ] Role mapping: owner vs staff → worksh.io roles; the BFF forwards, it does
      not decide

## Phase 3 — Pesanan & laporan dasar (M)

**Exit:** orders arrive and are fulfilled in-app; Beranda and Laporan show real
numbers.

- [ ] `GET /v1/orders` across channels (lapak, marketplace, chat-to-checkout),
      detail, `POST /v1/orders/:id/fulfil` (process, ship, resi)
- [ ] `GET /v1/home` — one aggregate: omzet hari ini, pesanan, stok menipis,
      open PasStok cards. Concurrent works-server calls with
      `tokio::try_join!`, a per-call timeout, and partial degradation (a slow
      section returns `null`, not a 500)
- [ ] `GET /v1/reports/summary` — omzet & untung, uang masuk vs untung, produk
      terlaris, stok value
- [ ] `seed/` generator: Toko Kopi Jasa, 90+ days of realistic sales
      (weekday/weekend, paydays, a promo week, perishables with lots),
      suppliers, past POs. Deterministic from a seed so the demo repeats

## Phase 4 — Data pipeline & forecast (L)

**Exit:** a daily forecast per SKU × location exists in BigQuery, with accuracy
tracked.

- [ ] `passtok-extractor`: backfill + incremental (cursor-paged
      `productMovements`), idempotent upserts into `raw`, cursor persisted,
      resumable after a crash
- [ ] `features.daily_demand`: zero-fill, stockout-day flag, closure days from
      `calendar_overrides`
- [ ] Scheduled query: `AI.FORECAST` (TimesFM), 14-day horizon with prediction
      interval → `forecast.forecasts`
- [ ] Cold-start fallback: category pooling / moving average, `confidence = low`
- [ ] Accuracy: holdout WAPE via `AI.EVALUATE`, compared against a 4-week moving
      average baseline. If we cannot beat the baseline, say so rather than ship
      the forecast
- [ ] Data-quality checks (row counts, freshness) with alerts

## Phase 5 — Risk engine & events (M)

**Exit:** `reorder.suggested` and `lot.expiry_risk` fire correctly for the seed
tenant.

- [ ] Reorder rule: days of cover, reorder point (lead time + review period +
      safety stock from the upper prediction interval), **expiry cap**, pack/MOQ
      rounding, grouped by supplier
- [ ] Expiry rule: FEFO sell-through simulation per lot, `at_risk_qty`, ranked
      actions with Rp recovered
- [ ] Production plan for recipe products (forecast − leftovers, component check)
- [ ] Waste feedback: repeated waste lowers the next suggestion (FR-36)
- [ ] Event schemas in `schemas/`, `typify` codegen, outbox publisher with
      `FOR UPDATE SKIP LOCKED`, DLQ handling, replay tool
      (`cargo run -p passtok-risk -- replay --from …`)
- [ ] Table-driven unit tests for every rule with `insta` golden files, plus
      `proptest` on the rounding and cap arithmetic

The expiry cap is the rule that makes PasStok different from a reorder-point
spreadsheet: never suggest more than can be sold before it expires. Test it
first.

## Phase 6 — Asisten Stok chat (L)

**Exit:** the demo's reorder story works end to end — card → question → edit →
approve → PO in worksh.io.

- [ ] `passtok` migrations: `threads`, `messages`, `suggestions`,
      `action_tokens`, `outbox`
- [ ] BFF: `GET /v1/assistant/thread`, `GET /v1/assistant/stream` (SSE +
      broadcast channel + Redis fan-out, `ping`, `resync`, `Last-Event-ID`),
      `POST /v1/assistant/messages`, read receipts
- [ ] `passtok-agent`: event → card with a Bahasa explanation; Q&A grounded
      **only** on the stored `suggestions.inputs`; read-only works-server
      access; refusal outside stock and procurement; hard turn cap
- [ ] Action tokens: Ed25519 signing in the agent, verification + single use in
      the executor, key in Secret Manager
- [x] Action-token verification, single use and quantity cap, with the result
      card and a supplier `wa.me` prefill carrying the **final** quantity
- [ ] Make it a real write: `createPurchaseOrder` → `submitPurchaseOrder`
      against worksh.io, and move the used-`jti` store into
      `passtok.action_tokens` so single use survives a crash mid-request
- [ ] Chat corrections → `calendar_overrides` (FR-31)
- [ ] `POST /v1/assistant/activate` — Aktifkan + Kesiapan answers (lead time,
      shelf life)
- [ ] `POST /v1/assistant/report` → moderation queue
- [ ] Auto-approve rules + executor path with a 1-hour cancel window — **P1**

## Phase 7 — Waste photo & AI vision (M)

**Exit:** photo → outlined items → confirmed write-off, with the photo stored as
evidence.

- [ ] `POST /v1/waste/photos` — V4 signed PUT URL, per-tenant prefix, retention
      policy on the bucket
- [ ] Server-side EXIF/GPS strip as a second line of defence (the PWA strips
      first)
- [ ] `POST /v1/waste/analyze` → agent → Gemini structured output (label, count,
      unit, condition, confidence, bbox) given the tenant's candidate products
      (perishable, in stock, near expiry first, with primary images)
- [ ] Grounded matching: accept **only** candidate ids; below threshold → top 3 + "Bukan produk toko"
- [ ] `POST /v1/waste/confirm` → executor `adjustStock` (lot-aware once
      works-core lands `lotId`), value at cost, `waste.recorded`
- [ ] Vision cache keyed by photo digest — the same photo must not be billed twice
- [ ] Evaluation set: 30–50 labelled photos from the demo shop; measure match
      precision and count error, checked in as a test

## Phase 8 — Waste destinations (L)

**Exit:** every waste item gets a legal destination; donation with receipt and
the daily special work; diversion score shows in reports.

- [ ] `GET /v1/waste/destinations` — options filtered by expiry state, highest
      recovery preselected, landfill always explicit and never default
- [ ] Destination write-offs with `dest:*` reasons; `waste.recorded` carries the
      destination
- [ ] Donations: `recipients` CRUD (partner + own), food-safety checklist as a
      hard gate, pickup window, WhatsApp notify, `donation.completed`
- [ ] Donation receipt PDF (`printpdf`), stored in GCS, served via signed URL
- [ ] Daily special: Gemini idea constrained to catalog + on-hand stock, with
      yield, price and recovery; approve → recipe + `createWorkOrder` + list on
      POS/lapak for the day
- [ ] `marts.waste_by_destination` + diversion score; `GET /v1/reports/waste`
- [ ] Weekly digest card: "dari N foto", diversion %

## Phase 9 — Notifications, hardening & demo (M)

**Exit:** the 7-minute demo runs three times in a row on staging without
intervention.

- [ ] `passtok-notifier`: Web Push (VAPID) + works_app push, escalation ladder,
      quiet hours, morning digest at 07.00 WIB
- [ ] WhatsApp utility template for urgent items (capped per day), deep-linking
      into the thread
- [ ] Observability: traces across `seller-svc` → executor → worksh.io;
      dashboards for event lag, Gemini latency and cost, error rate
- [ ] Security: tenant-isolation tests, token-replay test, prompt-injection
      tests on product names, rate limits, CSP headers
- [ ] Cost guardrails: Gemini daily spend cap with a graceful fallback,
      BigQuery bytes cap
- [ ] Demo kit: seed reset script, scripted event triggers, backup video
- [ ] Submission: README, architecture diagram, Google Cloud services list,
      impact numbers with sources

## Phase 10 — Sell-side waste features (post-hackathon, M)

- [ ] Markdown ladder (needs the worksh.io price override) on lapak + POS
- [ ] Bundling suggestions and bundle product creation
- [ ] "Diskon hari ini" feed for bikinlapak.com
- [ ] Surplus hand-off (deep link → partnership)
- [ ] Staff-meal destination

## Phase 11 — Recycle & reporting (post-hackathon, M)

- [ ] Used cooking oil log + sale as other income (`createJournalEntry`)
- [ ] Organic waste to BSF/compost partner pickup
- [ ] Packaging to bank sampah
- [ ] Damaged-on-arrival → supplier return / credit note (needs works-core)
- [ ] Monthly impact report PDF; CO₂e once an emission factor is chosen;
      "Toko Minim Sisa" badge rules

## Phase 12 — Scale & platform (post-hackathon, L)

- [ ] Outbox → Pub/Sub in works-consumer; retire extractor polling
- [ ] Multi-location transfers as an expiry action
- [ ] Image embeddings for candidate pre-ranking (FR-37)
- [ ] Seller credit profile groundwork from purchase and sales history
- [ ] Pricing tiers and paywall (WhatsApp nudges, auto-approve, reports)
