# Smoke test — PROCESSOR.SampleAnalyzer

Is `PROCESSOR.SampleAnalyzer` deployed? Written by `.apk.scripts/smoke.py` from the module's own shape; what each check means is in that script's header.

← [Wiki home](../README.md)

**❌ FAIL** — 2/3 checks, since 2026-11-12 00:16 EST

| Check | Target | Result | Detail |
|---|---|---|---|
| pushed | [APK.softapp.PROCESSOR.SampleAnalyzer](https://github.com/LikeDotAudio/APK.softapp.PROCESSOR.SampleAnalyzer.git) | ❌ | 1 commit(s) not pushed |
| container | `Worker-SAMPLE_ANALYZER` | ✅ | running, healthy |
| web | [SRC/core/Cargo.toml](https://apk.audio/APK%3ASoftapps/PROCESSOR/PROCESSOR.SampleAnalyzer/SRC/core/Cargo.toml) | ✅ | HTTP 200 |

Re-run: `python3 .apk.scripts/smoke.py PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer`

<!-- smoke:sig 1aa36bdbdd50 since 2026-11-12 00:16 EST -->
