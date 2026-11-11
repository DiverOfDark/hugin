FROM rust:1.99-bookworm AS builder
WORKDIR /app

# Build the dependency graph first so Docker can reuse this layer when only
# source files change. The dummy main keeps Cargo happy while compiling
# dependencies.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && printf 'fn main() {}\n' > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src target/release/deps/hugin* target/release/hugin*

COPY src ./src
COPY assets ./assets
COPY fixtures ./fixtures
RUN cargo build --release --locked

# glibc + libgcc only; TLS roots are compiled into the binary and the
# in-cluster CA comes from the service account mount.
FROM gcr.io/distroless/cc-debian12:nonroot
LABEL org.opencontainers.image.source="https://github.com/DiverOfDark/hugin" \
      org.opencontainers.image.description="Small server-rendered homelab dashboard for Kubernetes" \
      org.opencontainers.image.licenses="MIT"
COPY --from=builder /app/target/release/hugin /usr/local/bin/hugin
# Fewer malloc arenas keeps resident memory flat for a mostly idle process.
ENV MALLOC_ARENA_MAX=2
USER 65532:65532
EXPOSE 8080 8081
ENTRYPOINT ["/usr/local/bin/hugin"]
