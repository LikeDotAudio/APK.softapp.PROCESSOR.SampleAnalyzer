# Worker-SAMPLE_ANALYZER — the Scanalyzer DSP core as a worker on the bus (LOOSE-workers-convert-rest).
#   docker compose -f 'PODS/POD.workers/Docker/docker-compose.yml' up -d --build sample-analyzer
#   docker build -f 'PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer/Docker/Dockerfile' -t worker-sample-analyzer:local .
#
# ⚠️ THE BUILD CONTEXT IS THE REPO ROOT (`context: ../../../../../..` in the
#    compose file beside this one) — BY DESIGN, the same rule as the plugin and
#    broker images (.apk.scripts/plugin_standalone.py; DATABUS.Broker.MQTT's
#    Dockerfile): the crates name their siblings by git (the SDK by tag
#    `sdk-v0.9`, CAPTURE.Extractor-Engine by `rev`), so `git archive HEAD` of
#    this repository builds on its own; THE IMAGE builds the checkout instead,
#    patching those git names back to the copied local paths with `--config`,
#    so it never fetches GitHub and ships exactly the runner, contracts and
#    engine the rest of the bench runs; the UCS bundle (DATATYPE.soundfx.UCS,
#    package apk-ucs-bundle, by `rev`) is patched the same way. Every COPY source
#    below is a REAL repo-root path (PODS/README.md rule 1).

# ---------- build ----------
FROM rust:1.94-bookworm AS build
WORKDIR /src
ENV CARGO_TARGET_DIR=/src/target
COPY rust-toolchain.toml ./

# THE DESTINATION MIRRORS THE CHECKOUT, so relative paths resolve here as they
# do on disk. NODE.BareMetal/SRC/contracts is a SYMLINK to <repo>/contracts in
# a checkout; the two real directories land where it names.
COPY ["contracts/rust",   "./PODS/POD.THICK/NODE.BareMetal/SRC/contracts/rust"]
COPY ["contracts/tokens", "./PODS/POD.THICK/NODE.BareMetal/SRC/contracts/tokens"]
COPY ["PODS/POD.THICK/DISCOVERY.Engine/SRC/Cargo.toml", "./PODS/POD.THICK/DISCOVERY.Engine/SRC/Cargo.toml"]
COPY ["PODS/POD.THICK/DISCOVERY.Engine/SRC/RUNNER",     "./PODS/POD.THICK/DISCOVERY.Engine/SRC/RUNNER"]
# The analyzer's build-time siblings (see the header): the engine, patched to
# this path below, and the UCS bundle build.rs embeds.
COPY ["PODS/POD.THIN/Softapps/CAPTURE/CAPTURE.Extractor-Engine/Cargo.toml", "./PODS/POD.THIN/Softapps/CAPTURE/CAPTURE.Extractor-Engine/Cargo.toml"]
COPY ["PODS/POD.THIN/Softapps/CAPTURE/CAPTURE.Extractor-Engine/build.rs",   "./PODS/POD.THIN/Softapps/CAPTURE/CAPTURE.Extractor-Engine/build.rs"]
COPY ["PODS/POD.THIN/Softapps/CAPTURE/CAPTURE.Extractor-Engine/src",        "./PODS/POD.THIN/Softapps/CAPTURE/CAPTURE.Extractor-Engine/src"]
COPY ["PODS/POD.THIN/Softapps/DATATYPE/DATATYPE.soundfx.UCS/categories",             "./PODS/POD.THIN/Softapps/DATATYPE/DATATYPE.soundfx.UCS/categories"]
COPY ["PODS/POD.THIN/Softapps/DATATYPE/DATATYPE.soundfx.UCS/producer_synonyms.json", "./PODS/POD.THIN/Softapps/DATATYPE/DATATYPE.soundfx.UCS/producer_synonyms.json"]
COPY ["PODS/POD.THIN/Softapps/DATATYPE/DATATYPE.soundfx.UCS/Cargo.toml", "PODS/POD.THIN/Softapps/DATATYPE/DATATYPE.soundfx.UCS/build.rs", "./PODS/POD.THIN/Softapps/DATATYPE/DATATYPE.soundfx.UCS/"]
COPY ["PODS/POD.THIN/Softapps/DATATYPE/DATATYPE.soundfx.UCS/src", "./PODS/POD.THIN/Softapps/DATATYPE/DATATYPE.soundfx.UCS/src"]
# The tool: the analyzer crate (SRC/core: manifest, build script, schema,
# source) and the worker crate (SRC/).
ARG TOOL=PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer
# (SRC/core is the analyzer crate, SRC/ the worker; the one COPY carries both.)
COPY ["PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer/SRC",        "./PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer/SRC"]

# The three git names, patched to the copied paths (the SRC/.cargo/config.toml a
# checkout uses is not read here: cargo looks for config from the cwd, /src).
ARG SDK=https://github.com/LikeDotAudio/APK.plugin.SDK
ARG ENGINE=https://github.com/LikeDotAudio/APK.softapp.CAPTURE.Extractor-Engine
ARG UCS=https://github.com/LikeDotAudio/APK.softapp.DATATYPE.soundfx.UCS
RUN set -e; \
    P1="patch.\"$SDK\".apk-plugin-runner.path=\"PODS/POD.THICK/DISCOVERY.Engine/SRC/RUNNER\""; \
    P2="patch.\"$SDK\".apkaudio-contracts.path=\"PODS/POD.THICK/NODE.BareMetal/SRC/contracts/rust\""; \
    P3="patch.\"$ENGINE\".extractor_engine.path=\"PODS/POD.THIN/Softapps/CAPTURE/CAPTURE.Extractor-Engine\""; \
    P4="patch.\"$UCS\".apk-ucs-bundle.path=\"PODS/POD.THIN/Softapps/DATATYPE/DATATYPE.soundfx.UCS\""; \
    cargo build --release --locked --config "$P1" --config "$P2" --config "$P3" --config "$P4" \
        --manifest-path "$TOOL/SRC/Cargo.toml" --bin apk-worker-sample-analyzer; \
    cargo build --release --locked --config "$P1" --config "$P2" --config "$P3" --config "$P4" \
        --manifest-path "$TOOL/SRC/Cargo.toml" -p apk-plugin-runner --bin apk-plugin-health; \
    mkdir -p /out \
 && cp target/release/apk-worker-sample-analyzer target/release/apk-plugin-health /out/

# ---------- runtime ----------
FROM debian:bookworm-slim
# Not root: the worker opens no port and touches no device. ca-certificates
# because the analyzer links ureq (the CLI's --db-url upload); the worker never
# calls it, but a TLS client without roots is a trap for whoever does.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --uid 10001 --no-create-home --shell /usr/sbin/nologin worker
COPY --from=build /out/ /usr/local/bin/
USER worker
ENTRYPOINT ["apk-worker-sample-analyzer"]
