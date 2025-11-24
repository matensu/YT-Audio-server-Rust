FROM rust:1.76 as builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:buster-slim
COPY --from=builder /app/target/release/rust-audio-stream /usr/local/bin/rust-audio-stream
WORKDIR /app
RUN mkdir -p storage/tracks
EXPOSE 3000
CMD ["/usr/local/bin/rust-audio-stream"]
