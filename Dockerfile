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

# cargo-chef splits the build in two, so the dependencies (about half the
# compile) are a layer of their own that only a change to a manifest or the
# lockfile rebuilds; any other change starts from that layer and compiles
# the game crate and the server alone. In CI the layers are kept in
# GitHub's Actions cache (scope `rooms-server`, written on master by
# ci.yml's `rooms-cache` job), so a runner starting from nothing finds them.
FROM rust:1.98.1-bookworm AS chef
RUN cargo install cargo-chef --version 0.1.78 --locked
WORKDIR /src

# The recipe: every manifest and the lockfile, with the sources left out
# and the workspace's own versions masked, so a version bump or an edit
# to the code leaves it, and the dependency layer, as it was.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS build
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release -p bongbong-server --recipe-path recipe.json
COPY . .
RUN set -eux; \
    if cargo tree -p bongbong-server -e normal | grep -q sola; then \
      echo "raylib is in the room server's graph; the image must stay headless" >&2; \
      exit 1; \
    fi; \
    cargo build --release -p bongbong-server

# distroless/cc carries glibc and libgcc, which the release binary links
# against; rustls' `ring` needs no OpenSSL, so nothing else is wanted.
# The distroless family is what boo-run's images sit on too.
FROM gcr.io/distroless/cc-debian12

# /ws alone on 4848; /health, /ready and /metrics on the admin port 4850
# (the deployment routes only the first through its Ingress).
EXPOSE 4848/tcp 4850/tcp

COPY --from=build /src/target/release/bongbong-server /bongbong-server

# 0.0.0.0 because a container's loopback is its own. No `--insecure`: the
# flag only declares that nothing terminates TLS in front, and here the
# ingress does. `--max-rooms` is the manifest's to override.
ENTRYPOINT ["/bongbong-server"]
CMD ["--listen", "0.0.0.0:4848", "--admin-listen", "0.0.0.0:4850"]
