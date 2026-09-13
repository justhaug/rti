# RTI coordinator / worker image. Build from the repo root:
#   docker build -f deploy/Dockerfile -t rti .
FROM rust:1.96-bookworm AS build
WORKDIR /src
COPY . .
# DuckDB is compiled from source (bundled); expect ~5-10 minutes on first build.
RUN cargo build --release -p rti-cli -p rti-infra

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates git ffmpeg rclone curl bash \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/rti /usr/local/bin/rti
COPY --from=build /src/target/release/rti-oracle-watchdog /usr/local/bin/rti-oracle-watchdog
COPY --from=build /src/prompts /opt/rti/prompts
COPY --from=build /src/AGENTS.md /opt/rti/AGENTS.md
COPY --from=build /src/tracks /opt/rti/tracks
# The project root lives on the persistent volume; first boot seeds it.
ENV RTI_ROOT=/data/project
COPY deploy/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh
EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["serve"]
