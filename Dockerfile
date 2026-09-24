# The room server's image (docs/online-coop-prd.md §4.8): one Rust binary
# on distroless, built headless.
#
# Headless is what makes the image small and the build plain: the server
# takes the game crate with `default-features = false`
# (server/Cargo.toml), so raylib is not in the graph and the builder
# needs no cmake, no X11 and no GL packages (CLAUDE.md's "Headless
# build"). The server reads no `static/` assets either - `SHIPPED_MAPS`
# are embedded and a builder map travels inside the `Welcome` - so the
# final stage is the binary and nothing else.
#
# Pinned toolchain: keep in sync with .github/workflows/ci.yml,
# .github/actions/build-web/action.yml and devenv.nix's
# languages.rust.version.

FROM rust:1.97.1-bookworm AS build
WORKDIR /src
COPY . .

# The cache mounts make a rebuild on this machine cheap; in CI the layer
# cache does that job instead, so neither is load-bearing. The binary is
# copied out inside the same RUN because a cache mount is not part of the
# layer it was mounted into.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    set -eux; \
    if cargo tree -p bongbong-server -e normal | grep -q sola; then \
      echo "raylib is in the room server's graph; the image must stay headless" >&2; \
      exit 1; \
    fi; \
    cargo build --release -p bongbong-server; \
    cp target/release/bongbong-server /bongbong-server

# distroless/cc carries glibc and libgcc, which the release binary links
# against; rustls' `ring` needs no OpenSSL, so nothing else is wanted.
# The distroless family is what boo-run's images sit on too.
FROM gcr.io/distroless/cc-debian12

# /ws, /health and /metrics all sit behind this one port.
EXPOSE 4848/tcp

COPY --from=build /bongbong-server /bongbong-server

# 0.0.0.0 because a container's loopback is its own. No `--insecure`: the
# flag only declares that nothing terminates TLS in front, and here the
# ingress does. `--max-rooms` is the manifest's to override.
ENTRYPOINT ["/bongbong-server"]
CMD ["--listen", "0.0.0.0:4848"]
