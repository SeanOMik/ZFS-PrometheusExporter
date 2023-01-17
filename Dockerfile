FROM rust:alpine3.14 as builder

# update packages
RUN apk update
RUN apk add zfs-dev build-base pkgconfig

# Install rust toolchains
RUN rustup toolchain install stable
RUN rustup default stable

# Build dependencies only. Separate these for caches
RUN cargo install cargo-build-deps

# Copy application now
WORKDIR /app
COPY ./ /app

RUN cargo build-deps --release

# Build the release executable.
RUN cargo build --release

# Runner stage.
FROM alpine

# update packages
RUN apk update
RUN apk add zfs

ARG UNAME=zfs_promexporter
ARG UID=1000
ARG GID=1000

# Add user and copy the executable from the build stage.
RUN adduser --disabled-password --gecos "" $UNAME -s -G $GID -u $UID
COPY --from=builder --chown=$UID:$GID /app/target/release/zfs_promexporter /app/zfs_promexporter

USER $UNAME

WORKDIR /app

ENTRYPOINT [ "/app/zfs_promexporter" ]