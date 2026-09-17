# syntax=docker/dockerfile:1@sha256:ecfaec9ed6d810b56388c508f4121597bfbba70d41a6dfeee4d8cad5f295fc32

FROM rust:1.88.0-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0 AS teatro-builder

ARG TARGETARCH
ARG BUILD_JOBS=2

# Use distro-maintained SQLite instead of the older C copy inside libsqlite3-sys.
ENV LIBSQLITE3_SYS_USE_PKG_CONFIG=1
RUN rm -f /etc/apt/sources.list /etc/apt/sources.list.d/* \
    && printf '%s\n' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260917T000000Z bookworm main' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/20260917T000000Z bookworm-security main' \
      > /etc/apt/sources.list \
    && apt-get update \
    && apt-get install --no-install-recommends -y libsqlite3-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Embed Rust metadata for scanners; retain Cargo tree separately to show active dependencies.
RUN --mount=type=cache,id=teatro-docker-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    cargo install cargo-auditable --version 0.7.6 --locked --jobs "$BUILD_JOBS"

WORKDIR /source
COPY Cargo.toml Cargo.lock build.rs RUST_DEPENDENCY_LICENSES.tsv ./
COPY packaging/collect-rust-notices.py packaging/collect-rust-notices.py
COPY packaging/licenses packaging/licenses
COPY migrations ./migrations
COPY src ./src
COPY web/admin ./web/admin
COPY web/setup ./web/setup
COPY web/public ./web/public
COPY web/fonts ./web/fonts
RUN --mount=type=cache,id=teatro-docker-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=teatro-docker-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=teatro-docker-auditable-sqlite-target-${TARGETARCH},target=/source/target,sharing=locked \
    cargo fetch --locked \
    && cargo auditable build --release --locked --bin teatro --jobs "$BUILD_JOBS" \
    && install -Dm755 target/release/teatro /out/teatro \
    && readelf --sections /out/teatro > /out/sections.txt \
    && grep --fixed-strings '.dep-v0' /out/sections.txt \
    && readelf --dynamic /out/teatro > /out/dynamic.txt \
    && grep --fixed-strings '[libsqlite3.so.0]' /out/dynamic.txt \
    && python3 packaging/collect-rust-notices.py /out/rust-notices

# Build the pinned extractor as a separate executable with its source and notices.
FROM debian:trixie-20260824-slim@sha256:d7e12182ce18b85b93007c1dedf31f2d29e01ccf3182cc4017c709b6259bc132 AS innoextract-builder

ARG TARGETARCH
ARG TARGETPLATFORM
ARG BUILD_JOBS=2
ARG BUILDKIT_SBOM_SCAN_STAGE=true
ENV DEBIAN_FRONTEND=noninteractive \
    LC_ALL=C.UTF-8 \
    TZ=UTC

RUN rm -f /etc/apt/sources.list /etc/apt/sources.list.d/* \
    && printf '%s\n' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260917T000000Z trixie main' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260917T000000Z trixie-updates main' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/20260917T000000Z trixie-security main' \
      > /etc/apt/sources.list \
    && printf 'Acquire::Check-Valid-Until "false";\nAcquire::Retries "3";\n' \
      > /etc/apt/apt.conf.d/99teatro-snapshot \
    && apt-get update \
    && apt-get install --no-install-recommends -y \
      ca-certificates \
      cmake \
      coreutils \
      curl \
      findutils \
      g++ \
      grep \
      gzip \
      libboost-all-dev \
      libbz2-dev \
      liblzma-dev \
      libzstd-dev \
      make \
      sed \
      tar \
      xz-utils \
      zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY packaging/innoextract/version.env /build/version.env
RUN set -eu; \
    . /build/version.env; \
    curl --fail --location --proto '=https' --tlsv1.2 \
      "$INNOEXTRACT_SOURCE_URL" --output /build/innoextract-source.tar.gz; \
    printf '%s  %s\n' "$INNOEXTRACT_SOURCE_SHA256" /build/innoextract-source.tar.gz \
      | sha256sum --check --strict; \
    mkdir /build/innoextract; \
    tar --extract --gzip --file /build/innoextract-source.tar.gz \
      --directory /build/innoextract --strip-components=1; \
    find /build/innoextract -exec touch -h --date="@$INNOEXTRACT_SOURCE_DATE_EPOCH" {} +

RUN set -eu; \
    . /build/version.env; \
    export SOURCE_DATE_EPOCH="$INNOEXTRACT_SOURCE_DATE_EPOCH"; \
    cmake -S /build/innoextract -B /build/output \
      -DCMAKE_BUILD_TYPE=Release \
      -DBUILD_TESTS=ON \
      -DRUN_TESTS=OFF \
      -DSTRICT_USE=ON \
      -DUSE_LD=bfd \
      -DUSE_LTO=OFF \
      -DUSE_STATIC_LIBS=ON; \
    cmake --build /build/output --parallel "$BUILD_JOBS"; \
    cmake --build /build/output --target check; \
    /build/output/innoextract --version > /build/innoextract-version.txt; \
    grep --fixed-strings "innoextract $INNOEXTRACT_VERSION" /build/innoextract-version.txt; \
    grep --fixed-strings "to $INNOEXTRACT_MAX_INNO_SETUP_VERSION" /build/innoextract-version.txt

RUN set -eu; \
    . /build/version.env; \
    strip --strip-unneeded /build/output/innoextract; \
    mkdir -p /bundle/tools /bundle/third-party/innoextract/licenses \
      /bundle/third-party/innoextract/source; \
    install -m 0755 /build/output/innoextract /bundle/tools/innoextract; \
    (cd /bundle/tools && sha256sum innoextract > innoextract.sha256); \
    cp /build/innoextract-source.tar.gz \
      "/bundle/third-party/innoextract/source/innoextract-$INNOEXTRACT_COMMIT.tar.gz"; \
    cp /build/innoextract/LICENSE /bundle/third-party/innoextract/LICENSE; \
    cp /build/innoextract/README.md /bundle/third-party/innoextract/README.md; \
    cp /build/innoextract/CHANGELOG /bundle/third-party/innoextract/CHANGELOG; \
    cp /build/innoextract-version.txt /bundle/third-party/innoextract/VERSION.txt; \
    readelf --dynamic /bundle/tools/innoextract \
      | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p' \
      | LC_ALL=C sort > /bundle/third-party/innoextract/runtime-libraries.txt; \
    while IFS= read -r library; do \
      case "$TARGETARCH:$library" in \
        amd64:ld-linux-x86-64.so.2|arm64:ld-linux-aarch64.so.1|\
        *:libc.so.6|*:libdl.so.2|*:libm.so.6|*:libpthread.so.0|*:librt.so.1) ;; \
        *) printf 'Unexpected %s dynamic dependency: %s\n' "$TARGETARCH" "$library" >&2; exit 1 ;; \
      esac; \
    done < /bundle/third-party/innoextract/runtime-libraries.txt; \
    case "$TARGETARCH" in \
      amd64) build_base_manifest='sha256:abc9cb88a5587630d7f915f47b23b0668fe250fbfc6457aa4d52b534c1bbf73f' ;; \
      arm64) build_base_manifest='sha256:7215f78f35ffe58fe13f244fac9c4f21326d55187271fbb3e1a8aa5cc7e387ab' ;; \
      *) printf 'Unsupported target architecture: %s\n' "$TARGETARCH" >&2; exit 1 ;; \
    esac; \
    dpkg-query --show --showformat='${Package}\t${Version}\n' \
      | LC_ALL=C sort > /bundle/third-party/innoextract/build-packages.tsv; \
    for package in \
      libboost1.83-dev libbz2-dev libgcc-14-dev liblzma-dev \
      libstdc++-14-dev libzstd-dev zlib1g-dev; do \
        copyright="/usr/share/doc/$package/copyright"; \
        test -f "$copyright"; \
        cp -L "$copyright" "/bundle/third-party/innoextract/licenses/$package.copyright"; \
    done; \
    printf '%s\n' \
      '# innoextract build provenance' \
      '' \
      "- Repository: $INNOEXTRACT_REPOSITORY" \
      "- Commit: $INNOEXTRACT_COMMIT" \
      "- Source archive: $INNOEXTRACT_SOURCE_URL" \
      "- Source SHA-256: $INNOEXTRACT_SOURCE_SHA256" \
      "- Reported version: $INNOEXTRACT_VERSION" \
      "- Declared maximum Inno Setup version: $INNOEXTRACT_MAX_INNO_SETUP_VERSION" \
      '- Build base: debian:trixie-20260824-slim' \
      '- Build base index digest: sha256:d7e12182ce18b85b93007c1dedf31f2d29e01ccf3182cc4017c709b6259bc132' \
      "- Build base $TARGETPLATFORM manifest: $build_base_manifest" \
      '- Debian package snapshot: 2026-09-17T00:00:00Z' \
      '- Build mode: Release, GNU BFD linker, static Boost/liblzma/zlib/bzip2/zstd/libstdc++/libgcc, LTO disabled' \
      > /bundle/third-party/innoextract/PROVENANCE.md

FROM debian:trixie-20260824-slim@sha256:d7e12182ce18b85b93007c1dedf31f2d29e01ccf3182cc4017c709b6259bc132 AS runtime

RUN rm -f /etc/apt/sources.list /etc/apt/sources.list.d/* \
    && printf '%s\n' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260917T000000Z trixie main' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260917T000000Z trixie-updates main' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/20260917T000000Z trixie-security main' \
      > /etc/apt/sources.list \
    && printf 'Acquire::Check-Valid-Until "false";\nAcquire::Retries "3";\n' \
      > /etc/apt/apt.conf.d/99teatro-snapshot \
    && apt-get update \
    && apt-get upgrade --no-install-recommends -y \
    && apt-get install --no-install-recommends -y ca-certificates curl libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/* \
    && install -d -o 10001 -g 10001 /data \
    && mkdir -p /usr/share/doc/teatro
RUN dpkg-query --show --showformat='${Package}\t${Version}\n' > /tmp/runtime-packages \
    && LC_ALL=C sort /tmp/runtime-packages > /usr/share/doc/teatro/runtime-packages.tsv \
    && dpkg-query --show --showformat='${source:Package}=${source:Version}\n' > /tmp/runtime-sources \
    && LC_ALL=C sort -u /tmp/runtime-sources > /usr/share/doc/teatro/runtime-sources.txt \
    && rm /tmp/runtime-packages /tmp/runtime-sources \
    && find / -xdev -type f -perm /6000 -exec chmod a-s {} +

COPY --from=teatro-builder /out/teatro /opt/teatro/teatro
COPY --from=teatro-builder /out/rust-notices /usr/share/doc/teatro/rust
COPY LICENSE /usr/share/doc/teatro/LICENSE
COPY THIRD_PARTY_NOTICES.md /usr/share/doc/teatro/SOURCE-THIRD-PARTY-NOTICES.md
COPY web/public/platform-icons/LICENSE /usr/share/doc/teatro/PLATFORM-ICONS-CC0
COPY --from=innoextract-builder /bundle/tools /opt/teatro/tools
COPY --from=innoextract-builder /bundle/third-party/innoextract /usr/share/doc/teatro/innoextract
COPY packaging/THIRD_PARTY_NOTICES.md /usr/share/doc/teatro/THIRD_PARTY_NOTICES.md
COPY packaging/trivy-ignore.yaml /usr/share/doc/teatro/SECURITY-EXCEPTIONS.yaml
COPY web/public/icons.LICENSE /usr/share/doc/teatro/LUCIDE-ICONS-LICENSE
COPY web/fonts/AtkinsonHyperlegibleNext-OFL.txt /usr/share/doc/teatro/ATKINSON-HYPERLEGIBLE-NEXT-OFL

ENV TEATRO_BIND_ADDR=0.0.0.0:4440 \
    TEATRO_DATA_DIR=/data \
    TEATRO_DATABASE_URL=sqlite:///data/teatro.sqlite3 \
    TEATRO_DEFAULT_LIBRARY_ROOT=/data/roms \
    TEATRO_ASSET_ROOT=/data/assets \
    TEATRO_LOG_FORMAT=json \
    RUST_LOG=teatro=info,tower_http=info

USER 10001:10001
EXPOSE 4440
VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:4440/healthz >/dev/null || exit 1

ENTRYPOINT ["/opt/teatro/teatro"]
CMD ["serve"]

# Accompany runtime binaries with their exact Debian source, including patches.
# Keep build-only download tooling out of the runtime image.
FROM innoextract-builder AS runtime-sources
ARG BUILDKIT_SBOM_SCAN_STAGE=false
COPY --from=runtime /usr/share/doc/teatro/runtime-sources.txt /runtime-sources.txt
RUN sed 's/^deb /deb-src /' /etc/apt/sources.list > /etc/apt/sources.list.d/source.list \
    && apt-get update \
    && mkdir /sources \
    && cd /sources \
    && while IFS= read -r package; do \
      apt-get source --download-only --only-source "$package" || exit; \
    done < /runtime-sources.txt \
    && sha256sum ./* > SHA256SUMS

FROM runtime
COPY --from=runtime-sources /sources /usr/share/doc/teatro/debian-source
LABEL org.opencontainers.image.source="https://github.com/lodilorenzo/teatro" \
      org.opencontainers.image.licenses="CC-BY-NC-SA-4.0" \
      org.opencontainers.image.description="Teatro beta; third-party components retain their own licenses"
