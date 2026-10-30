# Worker-SAMPLE_ANALYZER — the Scanalyzer DSP core as a worker on the bus (LOOSE-workers-convert-rest).
#   docker compose -f 'PODS/POD.workers/Docker/docker-compose.yml' up -d --build sample-analyzer
#   docker build -f 'PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer/Docker/Dockerfile' -t worker-sample-analyzer:local .
#
# ⚠️ THE BUILD CONTEXT IS THE REPO ROOT (`context: ../../../../../..` in the
#    compose file beside this one): the worker links apkaudio-contracts and
#    apk-plugin-runner, and the analyzer itself reads two sibling
#    repositories at build time (build.rs): CAPTURE.Extractor-Engine (linked,
#    and hashed into ANALYZER_VERSION) and DATATYPE.soundfx.UCS (the category
#    bundle it embeds). Every COPY source below is a REAL repo-root path
#    (PODS/README.md rule 1). Same recipe as DATATYPE.wireless.CSV's worker.

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
# The analyzer's build-time siblings (see the header).
COPY ["PODS/POD.THIN/Softapps/CAPTURE/CAPTURE.Extractor-Engine/Cargo.toml", "./PODS/POD.THIN/Softapps/CAPTURE/CAPTURE.Extractor-Engine/Cargo.toml"]
COPY ["PODS/POD.THIN/Softapps/CAPTURE/CAPTURE.Extractor-Engine/src",        "./PODS/POD.THIN/Softapps/CAPTURE/CAPTURE.Extractor-Engine/src"]
COPY ["PODS/POD.THIN/Softapps/DATATYPE/DATATYPE.soundfx.UCS/categories",             "./PODS/POD.THIN/Softapps/DATATYPE/DATATYPE.soundfx.UCS/categories"]
COPY ["PODS/POD.THIN/Softapps/DATATYPE/DATATYPE.soundfx.UCS/producer_synonyms.json", "./PODS/POD.THIN/Softapps/DATATYPE/DATATYPE.soundfx.UCS/producer_synonyms.json"]
# The tool: the analyzer crate (manifest, build script, source) and the
# worker crate in its SRC/.
ARG TOOL=PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer
COPY ["PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer/Cargo.toml", "./PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer/Cargo.toml"]
COPY ["PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer/build.rs",   "./PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer/build.rs"]
COPY ["PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer/src",        "./PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer/src"]
COPY ["PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer/SRC",        "./PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer/SRC"]

RUN cargo build --release --locked --manifest-path "$TOOL/SRC/Cargo.toml" --bins \
 && mkdir -p /out \
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
