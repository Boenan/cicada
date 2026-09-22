# The shipped image (design §Distribution), built by `.github/workflows/artifacts.yml`.
#
# It contains **git** (#17). cicada shells out to git for every read and every
# write, so `scratch` is out however tempting a static binary makes it — an image
# without git is an image where cicada refuses at startup.
#
# The binary is not built here. It is built natively per architecture by the
# workflow and copied in, so this file never carries a cross-compilation
# toolchain and a multi-arch build does not compile Rust under emulation.

FROM alpine:3.22

# Docker's spelling of the architecture — `amd64` / `arm64`, supplied by buildx.
ARG TARGETARCH
ARG VERSION=dev

# `git` is the hard requirement. `ca-certificates` is for git's own HTTPS: cicada
# compiles its TLS roots in (webpki-roots), but git does not, and a push failing
# on certificate verification inside a release job is a bad way to find out.
RUN apk add --no-cache git ca-certificates

COPY dist/cicada-${TARGETARCH} /usr/local/bin/cicada

# Runs under emulation for the foreign architecture, which is the point: it
# proves the binary in this layer actually executes on the platform the manifest
# claims it does. A mismatched COPY is otherwise invisible until a user pulls it.
RUN cicada --version && git --version

# No ENTRYPOINT. This is a CI image before it is a CLI image: GitLab runs
# `script:` lines in a shell inside it (`ci/gitlab-ci.yml`), and an entrypoint is
# one more thing for a runner to have to override.
CMD ["cicada"]

LABEL org.opencontainers.image.title="cicada" \
      org.opencontainers.image.description="Release automation driven by conventional commits — versions, changelogs, tags and releases." \
      org.opencontainers.image.source="https://github.com/boenan/cicada" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0" \
      org.opencontainers.image.version="${VERSION}"
