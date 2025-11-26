# -----------------------------
# Stage 1: Build Rust application
# -----------------------------
FROM rust:1.86-slim-bullseye AS builder

# Installer les dépendances pour Rust (openssl)
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    ffmpeg \
    python3-pip \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app
COPY . .

# Build release
RUN cargo build --release

# -----------------------------
# Stage 2: Runtime minimal
# -----------------------------
FROM debian:bookworm-slim

# Installer runtime system dependencies
RUN apt-get update && apt-get install -y \
    ffmpeg \
    python3-pip \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Installer yt-dlp globalement
RUN python3 -m pip install --no-cache-dir yt-dlp

WORKDIR /usr/src/app
# Copier le binaire Rust compilé
COPY --from=builder /usr/src/app/target/release/rust-audio-stream .

EXPOSE 3000
CMD ["./rust-audio-stream"]
