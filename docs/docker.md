# Docker deployment and operations

[Documentation home](../README.md#documentation) · [Configuration](configuration.md)

- [Install](#install)
- [Storage](#storage)
- [Accounts and administration](#accounts-and-administration)
- [Network security](#network-security)
- [Backup and restore](#backup-and-restore)
- [Upgrade and rollback](#upgrade-and-rollback)
- [Troubleshooting](#troubleshooting)

## Install

Requirements: a 64-bit Linux Docker engine, Compose v2, curl, outbound access to
GHCR and enough local disk space for games and temporary work. Published beta
images support native amd64 and arm64. No Rust toolchain or local build is needed.
They are not a stable release or a guarantee of support for every Docker host.

### Use a published beta image

The [GHCR package page](https://github.com/users/lodilorenzo/packages/container/package/teatro)
lists published versions. There is no `latest` tag. The example below pins the
signed multi-platform index from this [successful publication run](https://github.com/lodilorenzo/teatro/actions/runs/35287351279)
and downloads its matching Compose file.

Install a [current Cosign release](https://docs.sigstore.dev/cosign/system_config/installation/)
with Sigstore bundle support. Review the [known findings and limits](image-publication.md#known-findings-and-scanner-limits)
before deploying. These images retain scoped vulnerability exceptions, not fixes
for every finding.

For a **fresh installation**, restrict port 4440 to your trusted network before
starting. Run these Bash commands in a parent directory where a new `teatro`
directory can be created. Existing deployments should use the
[upgrade instructions](#upgrade-and-rollback), not overwrite their configuration.

```bash
set -euo pipefail
mkdir teatro
cd teatro
curl --fail --location --output docker-compose.yml \
  https://raw.githubusercontent.com/lodilorenzo/teatro/c713e1e7c53581a365fc3f2bbca6975ca6ded9d7/docker-compose.yml
export TEATRO_IMAGE=ghcr.io/lodilorenzo/teatro@sha256:9bbeb09704c57a6b3a3dc7b44feac4ee45a515adab3e68f800657ed6ac9fd3f8
cosign verify "$TEATRO_IMAGE" \
  --certificate-identity 'https://github.com/lodilorenzo/teatro/.github/workflows/docker.yml@refs/heads/main' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
printf 'TEATRO_IMAGE=%s\n' "$TEATRO_IMAGE" > .env
docker compose config --quiet
docker compose pull
docker compose up --no-build -d
```

Docker selects amd64 or arm64 from the verified index. Keep the non-secret
`TEATRO_IMAGE` entry in `.env` beside `docker-compose.yml` and include both files
in backups. This prevents later commands from falling back to `teatro:local`.
Do not use `--build` for the published-image deployment.

The image includes the pinned `innoextract` sidecar, license notices and
corresponding Debian sources, so it is larger than the executable alone. It uses
Debian's patched SQLite library. The process runs as UID/GID `10001:10001`, listens
on port 4440 and handles Docker's stop signal. Compose supplies persistent storage,
the image's health check and `unless-stopped` restarts. It drops all capabilities
and prevents privilege gain. Keep these settings.

### Build from source

This alternative needs Git, BuildKit, network access to pinned build inputs and
enough memory for compilation. Use a separate source checkout and explicitly
select the local image rather than the registry deployment's `TEATRO_IMAGE`:

```bash
git clone https://github.com/lodilorenzo/teatro.git teatro-source
cd teatro-source
TEATRO_IMAGE=teatro:local docker compose build
TEATRO_IMAGE=teatro:local docker compose up --no-build -d
```

The [Dockerfile](../Dockerfile) builds and checks the same bundled sidecar.
Do not run this deployment alongside another server using the same data volume.
For running without Docker, see the [native source guide](development.md#run-from-source).

### Create the administrator

Open `http://YOUR_SERVER:4440/setup` on a trusted network immediately. Anyone who
can reach an unconfigured server can create its first administrator. Setup closes
after an administrator exists. Player and admin interfaces are at `/` and `/admin`.
For terminal-only setup, use the password-prompting command:

```bash
docker compose exec teatro /opt/teatro/teatro users create-admin --username admin
curl --fail http://127.0.0.1:4440/healthz
```

Do not put administrator passwords into the Compose file or a persistent `.env`.
Stop without deleting data with `docker compose down`. Do not add `--volumes`
or `-v` unless you intend to delete the stored library.

## Storage

The default named volume is `teatro-data`, mounted at `/data`. It contains SQLite,
its WAL/SHM files, the instance lock, `roms/`, `assets/`, and optional discovery and
GOG staging data. Keep the complete directory together, not just the database file.

Use a local filesystem, not NFS or SMB. Run one server per data set. Teatro takes
an exclusive instance lock and uses SQLite WAL; multiple replicas are unsupported.
Changing `TEATRO_DATA_VOLUME` selects another volume, it does not move existing data.

For a host-visible data directory, replace the service's volume mapping:

```yaml
volumes:
  - /srv/teatro:/data
```

Create a **new** directory with the expected ownership before starting:

```bash
sudo install -d -o 10001 -g 10001 /srv/teatro
```

Bind mounts replace the image's prepared permissions. Existing data must be
readable and writable by UID/GID 10001, including file creation, rename and deletion.
Back it up before changing ownership. Do not solve permission errors by running
the service as root or making the directory world-writable.

To keep an existing library separate, retain `/data` and add a writable mount:

```yaml
environment:
  TEATRO_DEFAULT_LIBRARY_ROOT: /library
volumes:
  - teatro-data:/data
  - /srv/games:/library
```

Use the [managed layout](library.md#scan-existing-files). Do not let another server
or file-management process modify this library while Teatro is operating on it.
The separate library must join every backup and restore.

## Accounts and administration

Use separate `readonly` accounts for players and client applications. Administrators can
change configuration and delete managed files. The last administrator cannot be
deleted or demoted. CLI commands prompt for passwords without echoing them:

```bash
docker compose exec teatro /opt/teatro/teatro users list
docker compose exec teatro /opt/teatro/teatro users create --username player --role readonly
docker compose exec teatro /opt/teatro/teatro users reset-password --username admin
docker compose exec teatro /opt/teatro/teatro report
```

Additional commands are `users set-role --username NAME --role admin|readonly`,
`users delete --username NAME`, and `users list --json`. Use `--help` for syntax.
CLI access is privileged host access, not an alternative HTTP login. Use the
running image's CLI; a different migration history is rejected. Review reports
and logs before sharing because operational data may identify your library.

[API tokens](api.md#authentication) provide scoped credentials for automation.
Password resets invalidate browser sessions, but do not replace explicit API-token
revocation. Treat database backups as sensitive; they contain integration secrets.

## Network security

The default port mapping listens on every host interface. Restrict it with a
firewall to trusted clients; do not forward it from the Internet. Basic Auth and
Bearer tokens are unencrypted over HTTP. Discovery does not add security.

For a reverse proxy running on the same host, put this in Compose's local `.env`:

```dotenv
TEATRO_HOST_PORT=127.0.0.1:4440
```

Configure an HTTPS reverse proxy for the whole origin, including `/api`, `/assets`
and the static interfaces. For example, with Caddy already installed and DNS/TLS
configured for your domain:

```caddyfile
games.example.com {
    reverse_proxy 127.0.0.1:4440
}
```

Keep the backend unreachable from untrusted peers. Set `TEATRO_TRUSTED_PROXY_IPS`
only to the exact direct peer IPs Teatro sees for your proxy. Docker networking may
make this a bridge address, not `127.0.0.1`. Never trust an entire LAN or arbitrary
forwarded headers. Set proxy body-size and timeout limits to accommodate the
[configured uploads](configuration.md#uploads-and-downloads); avoid logging
Authorization headers or request bodies. Verify health, login and a download
through HTTPS before allowing other clients.

## Backup and restore

Stop Teatro and all other writers before copying SQLite, every library root,
assets and deployment configuration as one backup set. Encrypt or restrict access
to backups. Keep the matching source revision or image alongside them.

For the **default named volume**, this Bash example uses the running deployment's
image to create a stopped backup. It works with a published or locally built
image and does not mount host directories into the helper:

```bash
set -euo pipefail
mkdir -p backups
chmod 700 backups
umask 077
BACKUP="backups/teatro-$(date -u +%Y%m%dT%H%M%SZ).tar.gz"
docker volume inspect teatro-data >/dev/null
CONTAINER=$(docker compose ps --quiet teatro)
test -n "$CONTAINER"
BACKUP_IMAGE=$(docker inspect --format '{{.Image}}' "$CONTAINER")
docker compose stop teatro
docker run --rm --network none --entrypoint tar \
  --mount type=volume,src=teatro-data,dst=/data,readonly \
  "$BACKUP_IMAGE" -C /data -czf - . > "$BACKUP"
tar -tzf "$BACKUP" >/dev/null
docker compose start teatro
```

If a command fails, investigate before restarting or treating the backup as valid.
Substitute your actual image and volume when customized. Back up external library
mounts and the Compose configuration while the same service remains stopped.

Restore into a **new empty volume**, keeping the original intact:

1. Stop Teatro and set `BACKUP_IMAGE` to the exact image reference saved with the
   backup. Pull it if needed. It must match the backup's migration history.
2. Choose an unused volume name, for example `teatro-restored`, and create it.
3. Extract only your trusted backup. A temporary root helper preserves recorded
   ownership; the Teatro service itself remains unprivileged:

   ```bash
   set -euo pipefail
   if docker volume inspect teatro-restored >/dev/null 2>&1; then
     echo 'Choose a new empty volume name before restoring.' >&2
     exit 1
   fi
   : "${BACKUP_IMAGE:?Set the image reference recorded with this backup}"
   docker volume create teatro-restored
   docker run --rm -i --network none --user 0:0 --entrypoint tar \
     --mount type=volume,src=teatro-restored,dst=/restore \
     "$BACKUP_IMAGE" -C /restore -xzf - < backups/YOUR_BACKUP.tar.gz
   ```

4. Restore any external roots from the same backup set. Set
   `TEATRO_DATA_VOLUME=teatro-restored` and the matching `TEATRO_IMAGE` in the local
   Compose `.env`, then run `docker compose up --no-build -d`. Do not run both copies against the same external roots.
5. Check health, login, user/game counts, covers and a file download. Keep the old
   data until verification succeeds. Restored discovery IDs must not be advertised
   by two instances on the same link.

## Upgrade and rollback

The public baseline is for fresh installations. Do not attach databases from
snapshots with a different migration history or manually edit SQLx migration records.

Before an upgrade, make and verify a stopped backup, note the current source SHA,
and retain the current image under another tag, such as `teatro:before-upgrade`.
For a published image, review its changes and security policy, then verify the new
index digest with Cosign as in the installation example. Update `TEATRO_IMAGE`
in your existing `.env`, preserving other settings. If it is also exported in
your shell, update or unset that variable so it does not override `.env`.
Then run:

```bash
docker compose config --quiet
docker compose pull
docker compose up --no-build -d
docker compose logs --tail=100 teatro
curl --fail http://127.0.0.1:4440/healthz
```

For a locally built image, check out the intended public source revision and run
`TEATRO_IMAGE=teatro:local docker compose build`, followed by
`TEATRO_IMAGE=teatro:local docker compose up --no-build -d` instead of pulling.

Startup applies embedded migrations and reconciles interrupted managed-file
operations. Check login, library counts and downloads before discarding the backup.
Rollback means restoring the **pre-upgrade data set** and starting its matching
old image. Running an older binary against a newer database is not a safe rollback.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Container exits or remains unhealthy | `docker compose logs --tail=100 teatro`; invalid environment values, migration errors, storage permissions and free space. |
| `/data` permission denied | Mount contents must be writable by UID/GID 10001. Stop and back up before repairing existing ownership. |
| Library appears empty | Verify the selected volume and mount paths, then platform folders and the scan report. A new volume starts a new database. |
| Another instance owns the data | Stop the competing server. Do not delete lock files to bypass an active process. |
| Login returns 401, 403 or 429 | Check credentials, effective role/token scope, and `Retry-After`. Avoid repeated password retries. |
| Upload returns 413 or 507 | Check Teatro and proxy limits, plus free space on both data and library filesystems. |
| Optional integration unavailable | Review [integration setup](integrations.md), its feature flag and credentials. Recreate the container after changing environment settings. |
| Build fails on a small host | Check memory, disk, native architecture and access to pinned download sources. Do not assume emulated builds are qualified. |

For an isolated local-image check, run
`scripts/test-docker-image.sh teatro:local`. It requires Bash and Python 3, creates
and removes its own volume and containers, and does not touch your running library.
