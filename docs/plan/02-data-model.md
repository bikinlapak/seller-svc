# 02 — Data model

Three stores, with a clear owner each:

1. **worksh.io tenant Postgres** — the system of record. Accounts, tenants,
   products, stock, lots, POs, journal. `seller-svc` reads and writes it only
   through works-server GraphQL, as the logged-in merchant.
2. **`passtok` schema**, in that same tenant Postgres — chat and PasStok state.
   New tables, listed below. Owned by worksh.io (migrations ship with it);
   written only through the BFF and the executor.
3. **BigQuery** — the analytics and forecasting side. Append-mostly, rebuilt
   from worksh.io at will; never a source of truth for a decision.

## `passtok` schema (per tenant)

| Table | Key columns |
| ----- | ----------- |
| `threads` | `id`, `kind` (`asisten_stok`), `created_at` |
| `messages` | `id`, `thread_id`, `author` (`agent` / `staff:<id>` / `system`), `type` (`text`, `reorder_card`, `expiry_card`, `result_card`, `digest`, `waste_card`, `special_card`, `donation_card`), `body`, `payload jsonb`, `ai_label`, `created_at`, `read_at` |
| `suggestions` | `id`, `kind`, `status` (`open` / `approved` / `edited` / `rejected` / `expired`), `inputs jsonb` (forecast run, on-hand, lead time, shelf life), `proposed jsonb`, `final jsonb`, `event_id` **unique** |
| `action_tokens` | `jti` pk, `suggestion_id`, `action`, `max_qty`, `expires_at`, `used_at` |
| `waste_records` | `id`, `photo_ids[]`, `product_id`, `lot_id`, `qty`, `unit`, `condition`, `destination`, `value_at_cost`, `vision_confidence`, `edited`, `actor`, `recorded_at` |
| `recipients` | `id`, `type` (`partner_food_bank` / `own`), `name`, `contact`, `address`, `pickup`, `active` |
| `donations` | `id` (`DON-YYYY-NNNN`), `recipient_id`, `lines jsonb`, `safety_checklist jsonb`, `pickup_window`, `picked_up_at`, `receipt_uri` |
| `markdowns` | `id`, `product_id`, `lot_id`, `steps jsonb`, `channels[]`, `starts_at`, `ends_at`, `then_destination`, `status` |
| `supplier_terms` | `supplier_id`, `product_id`, `lead_time_days`, `pack_size`, `min_order_qty` (until works-core has them) |
| `product_extras` | `product_id`, `shelf_life_days`, `perishable` (until works-core has them) |
| `calendar_overrides` | `date`, `location_id`, `closed`, `note` (from chat corrections, FR-31) |
| `auto_approve_rules` | `product_id`, `max_value`, `min_unedited_approvals`, `enabled` |
| `push_subscriptions` | `staff_id`, `endpoint`, `keys`, `platform`, `created_at` |
| `outbox` | `id`, `topic`, `payload`, `created_at`, `published_at` |

### Invariants to encode in the schema, not in handlers

- `suggestions.event_id` is unique → a replayed Pub/Sub delivery cannot create a
  second card for the same risk event. The agent upserts on it.
- `action_tokens.used_at` is set in the **same transaction** as the mutation
  record → single use survives a crash mid-request and two concurrent taps.
- `action_tokens.max_qty` is checked against the submitted quantity by the
  executor; an edit may only go down.
- `outbox.published_at IS NULL` is the publish queue. The publisher claims rows
  with `FOR UPDATE SKIP LOCKED`, publishes, then stamps the row. At-least-once
  to Pub/Sub plus an idempotent consumer, which is the honest version of
  "exactly once".
- `waste_records.photo_ids` references GCS objects under the tenant prefix; the
  row is the evidence link for the journal entry.

### Migrations

`sqlx` migrations in `sql/passtok/`, one file per change, forward-only. Every
migration is `CREATE SCHEMA IF NOT EXISTS passtok` plus schema-qualified DDL, so
it is safe to run against a tenant database that worksh.io also migrates. The
tables that exist only until works-core catches up (`supplier_terms`,
`product_extras`) carry a comment naming the works-core issue that retires them.

Run them from `passtok-extractor`'s startup path or a dedicated
`cargo run -p passtok-migrate` — not from the BFF, which should not hold DDL
privileges.

### Connection shape

`sqlx::PgPool`, one pool per tenant database, cached by tenant id with an LRU
bound. For the hackathon's single demo tenant this is one pool; the bound exists
so the shape does not need rewriting at ten tenants. Statement timeout set at
the pool level; `search_path` left alone and every query schema-qualified.

## BigQuery datasets

- **`raw`** — `movements`, `stock_snapshots`, `lots`, `products`, `suppliers`,
  `purchase_orders`. Partitioned by date, clustered by `tenant_id`.
- **`features`** — `daily_demand` (stockout days flagged), `calendar` (paydays,
  Ramadan/Lebaran, closures).
- **`forecast`** — `forecast_runs`, `forecasts` (point + interval), `accuracy`
  (WAPE).
- **`events`** — `decisions`, `waste`, `donations`, `markdowns`,
  `notifications`, landed by Pub/Sub BigQuery subscriptions.
- **`marts`** — `weekly_impact`, `waste_by_destination`, `diversion_score`,
  `suggestion_funnel`.

`features.daily_demand` is the one that decides forecast quality: zero-fill
missing days, flag stockout days so the model does not learn "demand was zero"
from "we had none to sell", and drop closure days from
`passtok.calendar_overrides`.

## Event envelope

One shape for every event, JSON Schema in `schemas/`, Rust types generated with
`typify` at build time:

```json
{
  "v": 1,
  "id": "01JABCDEF...",
  "type": "reorder.suggested",
  "tenant_id": "...",
  "occurred_at": "2026-10-15T02:00:00Z",
  "trace_id": "...",
  "payload": { }
}
```

Topics mirror the type: `reorder.suggested`, `lot.expiry_risk`,
`production.suggested`, `decision.recorded`, `waste.recorded`,
`donation.completed`, `markdown.scheduled`, `notification.sent`. Each gets a
dead-letter topic. Consumers tolerate unknown fields and must be idempotent on
`id`.

## worksh.io changes requested (works-core)

- [ ] `adjustStock` optional `lotId`; FEFO consumption when absent; reason codes
      `waste:*` and `dest:*`; evidence reference.
- [ ] Supplier `leadTimeDays`; supplier-product `packSize`, `minOrderQty`;
      product `shelfLifeDays` (then retire the `passtok` stand-ins).
- [ ] Time-boxed price override per product/lot, applied to storefront and POS.
- [ ] Supplier purchase return / credit note.
- [ ] Outbox → Pub/Sub bridge in works-consumer (replaces extractor polling,
      post-hackathon).
- [ ] Read-only MCP scope for the agent.

Until the first two land: write off without a lot id, and read lead time, pack
size, MOQ and shelf life from the `passtok` stand-in tables. Phase 5's reorder
rule must therefore treat these as *optional* inputs with a documented default
(lead time 3 days, pack size 1, MOQ 1) and say so on the card — a suggestion
built on a guessed lead time should admit it.
