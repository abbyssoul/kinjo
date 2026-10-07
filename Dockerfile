# syntax=docker/dockerfile:1
#
# Kinjo with both discovery backends. On Linux both browse through the host's
# avahi-daemon over the system D-Bus, so the container needs the host's D-Bus
# socket, not the host's network, to discover services. zeroconf links
# libavahi-client, so this is a glibc build rather than the static musl binary
# the Linux archives ship. See "Docker" in README.md for the run arguments.
#
# The release workflow passes RUST_VERSION from rust-toolchain.toml.
# rust-toolchain.toml is not copied in: rustup would then download the
# clippy, rustfmt and rust-analyzer components it lists.
ARG RUST_VERSION=1.94

FROM docker.io/library/rust:${RUST_VERSION}-slim-trixie AS build
RUN apt-get update && \
    apt-get install -y --no-install-recommends clang libclang-dev libavahi-client-dev && \
    rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
RUN cargo build --release --locked --features zeroconf && \
    install -D -m 0755 target/release/kinjo /out/kinjo

FROM docker.io/library/debian:trixie-slim
# openssh-client serves the default ssh commands; libnss-mdns lets them resolve
# the .local {hostname} through the host's avahi-daemon socket when it is
# mounted. The xdg-open commands cannot reach a host browser from inside the
# container.
RUN apt-get update && \
    apt-get install -y --no-install-recommends libavahi-client3 libnss-mdns openssh-client && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --create-home --uid 10001 kinjo
COPY --from=build /out/kinjo /usr/bin/kinjo
COPY LICENSE README.md /usr/share/doc/kinjo/
COPY actions/*.toml /etc/kinjo/commands/
USER kinjo
WORKDIR /home/kinjo
ENTRYPOINT ["/usr/bin/kinjo"]
