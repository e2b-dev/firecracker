FROM rust:1.89-slim AS builder
WORKDIR /build
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /build/target/release/firecracker /usr/local/bin/firecracker
ENTRYPOINT ["firecracker"]
