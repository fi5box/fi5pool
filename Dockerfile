FROM rust:slim-bullseye AS buildstage
WORKDIR /build
ENV PROTOC_NO_VENDOR 1
ENV OPENSSL_DIR=/usr
ENV OPENSSL_LIB_DIR=/usr/lib
ENV OPENSSL_INCLUDE_DIR=/usr/include
RUN rustup component add rustfmt && \
    apt-get update && \
    apt-get install -y --no-install-recommends clang lld musl-dev pkgconf libssl-dev && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*
COPY . /build/
RUN cargo build --release

FROM debian:bullseye-slim
RUN useradd -m fi5pool
USER fi5pool
COPY --from=buildstage /build/target/release/fi5pool /usr/bin/
CMD ["fi5pool"]
