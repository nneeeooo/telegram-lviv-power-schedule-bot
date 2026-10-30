# ==========================================
# Builder Stage
# ==========================================
FROM docker.io/library/rust:1.85-alpine3.21 AS builder

RUN apk add --no-cache musl-dev

WORKDIR /app

# Cache dependency compilation
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Build actual binary
COPY src ./src
RUN touch src/main.rs && cargo build --release

# ==========================================
# Runtime Stage (LTS Alpine 3.21)
# ==========================================
FROM docker.io/library/alpine:3.21

RUN apk add --no-cache ca-certificates tzdata font-dejavu

WORKDIR /app

COPY --from=builder /app/target/release/telegram-lviv-power-schedule-bot /app/telegram-lviv-power-schedule-bot

RUN mkdir -p /app/cache

VOLUME ["/app/cache"]

CMD ["/app/telegram-lviv-power-schedule-bot"]
