#!/usr/bin/env bash
# Part of the APK.audio project — http://APK.audio — made by Anthony Kuzub
# MIT Licence. Free, for everyone, for ever. Full text in LICENSE at the root.
#
# The one check for this repository, run by hand before a push (the monorepo's
# CI deliberately verifies nothing; see the root .github/workflows header):
#   ./check.sh            unit + contract tests of both crates
#   ./check.sh --live     ... then SRC/probe_job.sh against the running worker
# Covers: ~90 analyzer tests, the .PEAK schema contract (an analysed record and
# the worker's bus result both validate against schema/peak.schema.json), the
# upload allow-list rule, and the worker's job/result/topic tests.
set -euo pipefail
cd "$(dirname "$0")"
cargo test --quiet
cargo test --quiet --manifest-path SRC/Cargo.toml
echo "check.sh: analyzer + worker tests OK"
if [[ "${1:-}" == "--live" ]]; then
  SRC/probe_job.sh
fi
