FROM ubuntu:22.04
ARG BUILD_DATETIME
ARG VCS_REF

LABEL org.opencontainers.image.authors="dev@chainflip.io"
LABEL org.opencontainers.image.vendor="Chainflip Labs GmbH"
LABEL org.opencontainers.image.title="chainflip/chainflip-engine"
LABEL org.opencontainers.image.source="https://github.com/chainflip-io/chainflip-backend/blob/${VCS_REF}/ci/docker/development/chainflip-engine.Dockerfile"
LABEL org.opencontainers.image.revision="${VCS_REF}"
LABEL org.opencontainers.image.created="${BUILD_DATETIME}"
LABEL org.opencontainers.image.environment="development"
LABEL org.opencontainers.image.documentation="https://github.com/chainflip-io/chainflip-backend"

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Copy the runner bniary and the dylib files.
COPY engine-runner /usr/local/bin/chainflip-engine
COPY old-engine-dylib/libchainflip_engine_v*.so /usr/local/lib/
# This path is set in the rpath of the runner binary build.rs file.
COPY libchainflip_engine_v*.so /usr/local/lib/

WORKDIR /etc/chainflip

RUN chmod +x /usr/local/bin/chainflip-engine

RUN apt-get update \
    && apt-get install -y ca-certificates --no-install-recommends \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

CMD ["/usr/local/bin/chainflip-engine"]
