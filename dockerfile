# Dockerfile pour Fly.io
FROM rust:1.71-slim-bullseye as builder

# Installer les dépendances nécessaires
RUN apt-get update && apt-get install -y \
    libssl-dev \
    pkg-config \
    ffmpeg \
    python3-pip \
    && pip3 install yt-dlp \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app
COPY . .

# Compiler le binaire en release
RUN cargo build --release

# Étape finale
FROM debian:bullseye-slim

# Installer runtime essentials
RUN apt-get update && apt-get install -y \
    libssl3 \
    ffmpeg \
    python3-pip \
    && pip3 install yt-dlp \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/local/bin
COPY --from=builder /usr/src/app/target/release/rust-audio-stream .

# Port exposé pour Fly.io
EXPOSE 8080

# Commande par défaut
CMD ["./rust-audio-stream"]
