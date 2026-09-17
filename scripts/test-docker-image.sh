#!/usr/bin/env bash
# Tests an already-built local image. Never pulls or mounts operator data.
set -euo pipefail
IMAGE="${1:?Usage: scripts/test-docker-image.sh LOCAL_IMAGE}"
IMAGE_ID="$(docker image inspect --format '{{.Id}}' "$IMAGE")"
[[ "$(docker image inspect --format '{{.Config.User}}' "$IMAGE_ID")" == '10001:10001' ]]
PASSWORD="$(python3 -c 'import secrets; print(secrets.token_urlsafe(24))')"
WORK_DIR="$(mktemp -d)"
NAME="teatro-smoke-${WORK_DIR##*/}"
VOLUME=""
CONTAINER=""
cleanup() {
  if [[ -n "$CONTAINER" ]]; then
    docker logs "$CONTAINER" || true
    docker rm --force "$CONTAINER" >/dev/null || true
  fi
  if [[ -n "$VOLUME" ]]; then docker volume rm "$VOLUME" >/dev/null || true; fi
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT
VOLUME="$(docker volume create "$NAME")"
printf 'Testing %s with fresh volume %s\n' "$IMAGE_ID" "$VOLUME"
start() {
  CONTAINER="$(docker run --detach --pull=never --network=none --name "$NAME" \
    --mount "type=volume,src=$VOLUME,dst=/data" "$IMAGE_ID")"
  for _ in {1..60}; do
    [[ "$(docker inspect --format '{{.State.Running}}' "$CONTAINER")" == true ]]
    if docker exec "$CONTAINER" curl --fail --silent http://127.0.0.1:4440/healthz; then
      return
    fi
    sleep 1
  done
  printf 'Container did not become ready.\n' >&2
  return 1
}
check_api() {
  docker exec "$CONTAINER" curl --fail --silent \
    --user "admin:$PASSWORD" http://127.0.0.1:4440/api/users/me > "$WORK_DIR/me.json"
  docker exec "$CONTAINER" curl --fail --silent \
    --user "admin:$PASSWORD" http://127.0.0.1:4440/api/platforms > "$WORK_DIR/platforms.json"
  python3 - "$WORK_DIR" <<'PY'
import json
import pathlib
import sys
root = pathlib.Path(sys.argv[1])
assert json.loads((root / 'me.json').read_text())['username'] == 'admin'
assert json.loads((root / 'platforms.json').read_text())
PY
}
stop() {
  docker stop --time 30 "$CONTAINER" >/dev/null
  [[ "$(docker inspect --format '{{.State.ExitCode}}' "$CONTAINER")" == 0 ]]
  docker logs "$CONTAINER" > "$WORK_DIR/container.log" 2>&1
  cat "$WORK_DIR/container.log"
  if grep -Fq "$PASSWORD" "$WORK_DIR/container.log" \
    || grep -Eiq 'panicked at|permission denied' "$WORK_DIR/container.log"; then
    printf 'Unsafe or failed startup/shutdown found in container log.\n' >&2
    return 1
  fi
  docker rm "$CONTAINER" >/dev/null
  CONTAINER=""
}
start
# Default identity can create, rename, and remove files on a brand-new volume.
docker exec "$CONTAINER" sh -ec '
  test "$(id -u):$(id -g)" = 10001:10001
  test "$(stat -c %u:%g /data)" = 10001:10001
  touch /data/write-check; mv /data/write-check /data/rename-check; rm /data/rename-check
  cd /opt/teatro/tools; sha256sum --check --strict innoextract.sha256
  ./innoextract --version
'
[[ "$(docker exec "$CONTAINER" curl --silent --output /dev/null --write-out '%{http_code}' \
  http://127.0.0.1:4440/api/users/me)" == 401 ]]
docker exec --env "TEATRO_ADMIN_PASSWORD=$PASSWORD" "$CONTAINER" \
  /opt/teatro/teatro users create-admin --username admin
check_api
stop
# A replacement container must authenticate against the same persisted database.
start
check_api
stop
printf 'Docker smoke passed: non-root writes, sidecar, auth, container replacement, persistence, clean shutdown.\n'
