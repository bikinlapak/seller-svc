# 07 — Risks and open questions

## Risks

| Risk | Mitigation |
| ---- | ---------- |
| Scope is large for a hackathon | Hard cut line after Phase 9; Phases 1–3 reuse existing worksh.io patterns; build the demo path first, breadth second |
| works-core changes block PasStok | `passtok` stand-in tables for lead time, shelf life, pack size; lot-less write-off until `lotId` lands; every stand-in carries the works-core issue that retires it |
| Gemini latency or cost in the demo | Pre-generate card text at event time; cache vision results per photo digest; small model for chat; daily spend cap with a graceful fallback |
| Demo data looks fake | Seed from realistic patterns and real shop photos; say on stage that it is simulated |
| PWA camera and push differ across phones | Test on the demo phone early; file-input fallback for camera (frontend plan, Phase 7) |

### Rust-specific risks

| Risk | Mitigation |
| ---- | ---------- |
| No ADK for Rust — the agent loop is ours to write | Keep the loop small: fixed read-only tools, hard turn cap, structured output. Write the refusal and injection tests before the loop (Phase 6), so the boundary is defined by tests rather than by framework defaults |
| Immature GCP crates (Pub/Sub, BigQuery, Storage are community-maintained) | Wrap each behind a `passtok-core` module with our own trait, so swapping to REST is a one-file change. Pin versions; `cargo deny` watches advisories |
| `aws-lc-sys` creeping in and breaking the build | CI gate: `cargo tree -i aws-lc-sys` must be empty. The usual cause is a transitive `rustls` default feature |
| Compile times slowing the loop at six crates | `cargo-chef` in Docker, `sccache` locally, `cargo check` in the inner loop; split tests so `cargo test -p passtok-risk` stays fast |
| `sqlx` compile-time checks need a schema | `SQLX_OFFLINE=true` with `.sqlx/` checked in; refresh via `cargo sqlx prepare` in a `just` recipe |
| One engineer, six deploy units | Keep them in one workspace and one Dockerfile with a `CRATE` arg; defer splitting the agent's vision path into its own service (the Notion plan already folds `vision` into `agent` for v1) |

## Open questions

Carried from the Notion plan, with what each one blocks:

- [ ] **Hackathon deadline and submission format** — sets the dates for every
      phase. Nothing else can be scheduled until this lands.
- [ ] **Does onboarding (Phase 1) need to be in the demo**, or can the demo
      start from a seeded shop? If seeded, Phase 1 drops behind Phase 4 and
      roughly three days come back.
- [ ] **Seller app as a new app, or a "bikinlapak mode" inside works_app
      (Flutter) later?** Affects nothing in Phases 0–9; affects whether the BFF
      stays browser-shaped or grows a mobile-shaped variant.
- [ ] **Gemini model choice per task** (chat, vision, daily special) and region
      availability → ADR-0006. Blocks the cost guardrail numbers in Phase 9.
- [ ] **Partner food bank for the demo pickup** — blocks the donation half of
      Phase 8; `recipients` of type `own` is the fallback.
- [ ] **Who reviews Bahasa copy and food-safety wording** — the food-safety
      checklist is a gate on a donation, so the wording is a correctness
      question, not a polish question.

### Added by this plan

- [ ] **Which Postgres do the `passtok` migrations run against in staging**, and
      who owns running them — works-core's migration pipeline, or
      `cargo run -p passtok-migrate`? ADR-0003 records the ownership; the
      mechanics are still open.
- [ ] **Is there a works-server read-only scope today** for the agent, or does
      Phase 6 ship with a read-only *account* as a stand-in until the MCP scope
      lands?
- [ ] **Default lead time, pack size and MOQ** when `supplier_terms` has no row.
      The plan assumes 3 days / 1 / 1 and says so on the card; confirm with
      someone who has ordered from these suppliers.
- [ ] **Vision confidence threshold** for auto-match vs "top 3 + Bukan produk
      toko" — needs the labelled photo set from Phase 7 before it can be set
      honestly.
