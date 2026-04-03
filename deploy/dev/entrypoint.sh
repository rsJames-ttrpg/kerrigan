#!/bin/bash
set -e

export PATH="/opt/kerrigan/bin:$PATH"

# Ensure data directories exist (host path mounts may be empty)
mkdir -p /data/artifacts 2>/dev/null || true

echo "=== starting overseer ==="
/opt/kerrigan/bin/overseer /opt/kerrigan/config/overseer.toml &
OVERSEER_PID=$!

# Wait for Overseer's TCP port to accept connections.
# There is no /health endpoint — we check that the port is listening.
echo "waiting for overseer on port 3100..."
for i in $(seq 1 30); do
  if curl -sf http://localhost:3100/api/jobs/definitions > /dev/null 2>&1; then
    echo "overseer ready (pid $OVERSEER_PID)"
    break
  fi
  if ! kill -0 "$OVERSEER_PID" 2>/dev/null; then
    echo "ERROR: overseer exited unexpectedly"
    exit 1
  fi
  sleep 1
done

# If overseer never became ready, bail
if ! curl -sf http://localhost:3100/api/jobs/definitions > /dev/null 2>&1; then
  echo "ERROR: overseer did not become ready after 30s"
  kill "$OVERSEER_PID" 2>/dev/null || true
  exit 1
fi

echo "=== starting creep ==="
/opt/kerrigan/bin/creep &
CREEP_PID=$!
echo "creep started (pid $CREEP_PID)"

echo "=== starting queen ==="
# Dynamic name avoids UNIQUE constraint on re-registration with persisted DB
export QUEEN_NAME="hatchery-$(hostname)"
exec /opt/kerrigan/bin/queen --config /opt/kerrigan/config/hatchery.toml
