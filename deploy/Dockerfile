# Pii daemon image (spec 104). Build from the repository root:
#   docker build -f deploy/Dockerfile -t pii .
# Run (store, runtime dir and log live on the /data volume):
#   docker run -d --name pii -v pii-data:/data -e PII_MODEL=ollama:granite4.2:3b pii
# Probe (non-zero when the daemon is not answering):
#   docker exec pii pii health --json
# The build produces a statically linked musl binary, so the runtime image needs
# no libc.

FROM rust:1-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends clang musl-tools \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add x86_64-unknown-linux-musl
WORKDIR /src
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl -p pii-cli

FROM alpine:3.20
COPY --from=build /src/target/x86_64-unknown-linux-musl/release/pii /usr/local/bin/pii
COPY examples/pii.production.toml /etc/pii/pii.toml
RUN adduser -D -h /data pii && mkdir -p /data/run /data/store /data/logs && chown -R pii /data
USER pii
ENV PII_RUNTIME_DIR=/data/run PII_CONFIG=/etc/pii/pii.toml
VOLUME /data
WORKDIR /data
HEALTHCHECK --interval=30s --timeout=5s CMD ["pii", "health", "--json"]
ENTRYPOINT ["pii", "daemon", "--foreground", "--store", "/data/store/store.redb", \
    "--workspace", "/data", "--log-file", "/data/logs/pii.log", \
    "--log-rotate-bytes", "10485760", "--log-keep-files", "5"]
