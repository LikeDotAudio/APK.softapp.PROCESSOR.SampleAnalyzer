# 🛣 Improvements — PROCESSOR.SampleAnalyzer (from the 2026-10-30 architecture audit)

What this repo should change to meet the standard, most valuable first. Source: [🔍Audits/ARCHITECTURE-AUDIT.md](../%F0%9F%94%8DAudits/ARCHITECTURE-AUDIT.md).
← [Roadmap](INDEX.md)

| # | Improvement | Category | Effort (S/M/L) | Evidence |
|---|---|---|---|---|
| 1 | Pin the UCS bundle by `rev` once DATATYPE.soundfx.UCS ships as a cargo package (`links`, `DEP_APK_UCS_DIR`) — Waiting on DATATYPE.soundfx.UCS (handoff filed); then `Docker/Dockerfile` patches it with `--config` like the engine. | Coupling & Cohesion | S | `build.rs` `ucs_root()` |

Everything else is delivered — see [🌍Publishing/DELIVERY.md](../%F0%9F%8C%8DPublishing/DELIVERY.md).
