# the rust version in `./rust-toolchain.toml` is 1.95
FROM lukemathwalker/cargo-chef:latest-rust-1.95.0 AS chef
WORKDIR /app/backend
ENV SQLX_OFFLINE=true

# planner
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# dep builder
FROM chef AS builder
COPY --from=planner /app/backend/recipe.json recipe.json

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        libclang-dev \
        libssl-dev \
        libxml2-dev \
        libxmlsec1-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN cargo chef cook --locked --release --recipe-path recipe.json

# normal build
COPY . .
ARG GIT_VERSION=unknown
ENV GIT_VERSION=$GIT_VERSION
RUN cargo build --locked --release --workspace

# runtime
FROM debian:stable-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
    && groupadd --system --gid 10001 app \
    && useradd --system --uid 10001 --gid app --no-create-home app

WORKDIR /app
USER 10001:10001

FROM runtime AS minilith
COPY --from=builder /app/backend/target/release/minilith /app/minilith
EXPOSE 8000
ENTRYPOINT ["/app/minilith"]

FROM runtime AS fed-auth
RUN apt-get install -y --no-install-recommends \
        ca-certificates

COPY --from=builder /app/backend/target/release/fed-auth /app/fed-auth
EXPOSE 8001
ENTRYPOINT ["/app/fed-auth"]

FROM runtime AS transactions
COPY --from=builder /app/backend/target/release/transactions /app/transactions
EXPOSE 8002
ENTRYPOINT ["/app/transactions"]
