FROM ubuntu:24.04

# Runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    git \
    zstd \
    && rm -rf /var/lib/apt/lists/*

# GitHub CLI (gh)
RUN curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
      -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
      > /etc/apt/sources.list.d/github-cli.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends gh \
    && rm -rf /var/lib/apt/lists/*

# Buck2 (pinned to 2026-01-19 release matching .buckconfig)
# gh requires auth for release downloads, so use the direct URL pattern.
# The release asset URL is deterministic: github.com/facebook/buck2/releases/download/<tag>/<asset>
RUN curl -fsSL "https://github.com/facebook/buck2/releases/download/2026-01-19/buck2-x86_64-unknown-linux-gnu.zst" \
      | zstd -d > /usr/local/bin/buck2 \
    && chmod +x /usr/local/bin/buck2

# Non-root user (Claude CLI refuses --dangerously-skip-permissions as root)
RUN useradd -m -s /bin/bash kerrigan

# Create directory structure
RUN mkdir -p /opt/kerrigan/bin /opt/kerrigan/drones /opt/kerrigan/config /data/artifacts \
    && chown -R kerrigan:kerrigan /data

# Copy pre-built binaries from staging dir (populated by deploy/dev/build.sh)
COPY deploy/dev/.stage/bin/overseer   /opt/kerrigan/bin/overseer
COPY deploy/dev/.stage/bin/queen      /opt/kerrigan/bin/queen
COPY deploy/dev/.stage/bin/creep      /opt/kerrigan/bin/creep
COPY deploy/dev/.stage/drones/claude-drone /opt/kerrigan/drones/claude-drone

# Copy container-specific configs
COPY deploy/dev/overseer.toml   /opt/kerrigan/config/overseer.toml
COPY deploy/dev/hatchery.toml   /opt/kerrigan/config/hatchery.toml

# Copy entrypoint
COPY deploy/dev/entrypoint.sh   /opt/kerrigan/entrypoint.sh

USER kerrigan

# Expose Overseer HTTP port
EXPOSE 3100

# Data volume
VOLUME /data

ENTRYPOINT ["/opt/kerrigan/entrypoint.sh"]
