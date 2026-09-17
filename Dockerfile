# syntax=docker/dockerfile:1

FROM rust:1.88.0-bullseye@sha256:b315f988b86912bafa7afd39a6ded0a497bf850ec36578ca9a3bdd6a14d5db4e AS teatro-builder

WORKDIR /source
COPY Cargo.toml Cargo.lock build.rs ./
COPY migrations ./migrations
COPY src ./src
COPY web/admin ./web/admin
COPY web/setup ./web/setup
COPY web/public ./web/public
COPY web/fonts ./web/fonts
RUN --mount=type=cache,id=teatro-docker-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=teatro-docker-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=teatro-docker-target,target=/source/target,sharing=locked \
    cargo build --release --locked --bin teatro \
    && install -Dm755 target/release/teatro /out/teatro

# Build the pinned extractor as a separate executable with its source and notices.
FROM debian:bullseye-20260623-slim@sha256:f18adf4e1d04b1d8ba48025b8e35003f4c748ddd3dd8e875fe4e7d9a9c0dec84 AS innoextract-builder

ARG TARGETARCH
ARG TARGETPLATFORM
ENV DEBIAN_FRONTEND=noninteractive \
    LC_ALL=C.UTF-8 \
    TZ=UTC

RUN rm -f /etc/apt/sources.list /etc/apt/sources.list.d/* \
    && printf '%s\n' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260623T000000Z bullseye main' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260623T000000Z bullseye-updates main' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/20260623T000000Z bullseye-security main' \
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
    cmake --build /build/output --parallel; \
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
      amd64) build_base_manifest='sha256:7d5a9679452f9a25d9c8ef2fcb3b9ba0cd1653799a998591292aec1679fad7a2' ;; \
      arm64) build_base_manifest='sha256:870eec5563fbee751e2c7a47548dc233b0d4f9363ed982c62fad914077dd5c6e' ;; \
      *) printf 'Unsupported target architecture: %s\n' "$TARGETARCH" >&2; exit 1 ;; \
    esac; \
    dpkg-query --show --showformat='${Package}\t${Version}\n' \
      | LC_ALL=C sort > /bundle/third-party/innoextract/build-packages.tsv; \
    for package in \
      libboost1.74-dev libbz2-dev libgcc-10-dev liblzma-dev \
      libstdc++-10-dev zlib1g-dev; do \
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
      '- Build base: debian:bullseye-20260623-slim' \
      '- Build base index digest: sha256:f18adf4e1d04b1d8ba48025b8e35003f4c748ddd3dd8e875fe4e7d9a9c0dec84' \
      "- Build base $TARGETPLATFORM manifest: $build_base_manifest" \
      '- Debian package snapshot: 2026-06-23T00:00:00Z' \
      '- Build mode: Release, GNU BFD linker, static Boost/liblzma/zlib/bzip2/libstdc++/libgcc, LTO disabled' \
      > /bundle/third-party/innoextract/PROVENANCE.md

FROM debian:bullseye-20260623-slim@sha256:f18adf4e1d04b1d8ba48025b8e35003f4c748ddd3dd8e875fe4e7d9a9c0dec84

RUN rm -f /etc/apt/sources.list /etc/apt/sources.list.d/* \
    && printf '%s\n' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260623T000000Z bullseye main' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260623T000000Z bullseye-updates main' \
      'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/20260623T000000Z bullseye-security main' \
      > /etc/apt/sources.list \
    && printf 'Acquire::Check-Valid-Until "false";\nAcquire::Retries "3";\n' \
      > /etc/apt/apt.conf.d/99teatro-snapshot \
    && apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && install -d -o 10001 -g 10001 /data

COPY --from=teatro-builder /out/teatro /opt/teatro/teatro
COPY --from=innoextract-builder /bundle/tools /opt/teatro/tools
COPY --from=innoextract-builder /bundle/third-party/innoextract /usr/share/doc/teatro/innoextract
COPY packaging/THIRD_PARTY_NOTICES.md /usr/share/doc/teatro/THIRD_PARTY_NOTICES.md
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
