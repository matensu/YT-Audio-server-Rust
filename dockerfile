# Étape 1 : builder
FROM rust:1.77 as builder

WORKDIR /usr/src/app

# Copier les fichiers Cargo
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Compiler en release
RUN cargo build --release

# Étape 2 : image minimale pour exécution
FROM debian:bullseye-slim

WORKDIR /usr/src/app

# Copier le binaire depuis le builder
COPY --from=builder /usr/src/app/target/release/myapp ./myapp

# Exposer le port attendu par Fly.io
EXPOSE 8080

# Lancer le binaire
CMD ["./myapp"]
