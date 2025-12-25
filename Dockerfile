# Stage 1: Development base with all dependencies
FROM rust:slim-bookworm AS dev

RUN apt-get update && apt-get install -y \
    libavcodec-dev \
    libavformat-dev \
    libavutil-dev \
    libswscale-dev \
    libavfilter-dev \
    libswresample-dev \
    libavdevice-dev \
    libturbojpeg0-dev \
    pkg-config \
    clang \
    nasm \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

CMD ["cargo", "run", "-p", "bucket-streamer"]

# Stage 2: Builder
FROM dev AS builder

COPY Cargo.toml ./
COPY crates ./crates

RUN cargo build --release -p bucket-streamer

# Stage 3: Production runtime
FROM debian:12-slim AS prod

RUN apt-get update && apt-get install -y \
    libavcodec59 \
    libavformat59 \
    libavutil57 \
    libswscale6 \
    libturbojpeg0 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /workspace/target/release/bucket-streamer /usr/local/bin/

EXPOSE 3000

CMD ["bucket-streamer"]
