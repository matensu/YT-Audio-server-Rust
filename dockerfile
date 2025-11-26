# -----------------------------
# Stage 1: Build Rust application
# -----------------------------
FROM rust:1.86-slim-bullseye AS builder

# Dépendances nécessaires pour compiler et yt-dlp
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    ffmpeg \
    python3 \
    python3-pip \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app
COPY . .

# Compilation en release (limiter la parallélisation pour builders avec peu de RAM)
RUN cargo build --release -j 1

# -----------------------------
# Stage 2: Runtime
# -----------------------------
FROM debian:bookworm-slim

# Dépendances runtime
RUN apt-get update && apt-get install -y \
    libssl3 \
    ffmpeg \
    python3 \
    python3-pip \
    && rm -rf /var/lib/apt/lists/*

# Installer yt-dlp via pip
RUN pip3 install --no-cache-dir yt-dlp

WORKDIR /usr/src/app

# Copier le binaire compilé
COPY --from=builder /usr/src/app/target/release/rust-audio-stream .

EXPOSE 3000
CMD ["./rust-audio-stream"]
