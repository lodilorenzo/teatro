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

Requirements: a 64-bit Linux Docker engine, BuildKit, Compose v2, outbound access
for the image build, and enough local disk space for games and temporary work.
The recipe targets native amd64 and arm64. Neither a published image nor
release-qualified support for every Docker host is available yet.

From the repository root:

```bash
docker compose config --quiet
docker compose up --build -d
docker compose ps
docker compose logs --tail=100 teatro
```

The root [Dockerfile](../Dockerfile) builds Teatro and a pinned `innoextract`
sidecar, checks the sidecar, and retains its notices. The runtime process runs
as UID/GID `10001:10001`, listens on port 4440, and handles Docker's stop signal.
The [Compose file](../docker-compose.yml) supplies the persistent volume, health
check inherited from the image, and `unless-stopped` restart policy.

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

For the **default** image and named volume, this Bash example creates a stopped
backup without mounting host directories into a helper container:

```bash
set -euo pipefail
mkdir -p backups
chmod 700 backups
umask 077
BACKUP="backups/teatro-$(date -u +%Y%m%dT%H%M%SZ).tar.gz"
docker volume inspect teatro-data >/dev/null
docker compose stop teatro
docker run --rm --network none --entrypoint tar \
  --mount type=volume,src=teatro-data,dst=/data,readonly \
  teatro:local -C /data -czf - . > "$BACKUP"
tar -tzf "$BACKUP" >/dev/null
docker compose start teatro
```

If a command fails, investigate before restarting or treating the backup as valid.
Substitute your actual image and volume when customized. Back up external library
mounts and the Compose configuration while the same service remains stopped.

Restore into a **new empty volume**, keeping the original intact:

1. Stop Teatro and select an image matching the backup's migration history.
2. Choose an unused volume name, for example `teatro-restored`, and create it.
3. Extract only your trusted backup. A temporary root helper preserves recorded
   ownership; the Teatro service itself remains unprivileged:

   ```bash
   set -euo pipefail
   if docker volume inspect teatro-restored >/dev/null 2>&1; then
     echo 'Choose a new empty volume name before restoring.' >&2
     exit 1
   fi
   docker volume create teatro-restored
   docker run --rm -i --network none --user 0:0 --entrypoint tar \
     --mount type=volume,src=teatro-restored,dst=/restore \
     teatro:local -C /restore -xzf - < backups/YOUR_BACKUP.tar.gz
   ```

4. Restore any external roots from the same backup set. Set
   `TEATRO_DATA_VOLUME=teatro-restored` in the local Compose `.env`, then run
   `docker compose up -d`. Do not run both copies against the same external roots.
5. Check health, login, user/game counts, covers and a file download. Keep the old
   data until verification succeeds. Restored discovery IDs must not be advertised
   by two instances on the same link.

## Upgrade and rollback

The public baseline is for fresh installations. Do not attach databases from
snapshots with a different migration history or manually edit SQLx migration records.

Before an upgrade, make and verify a stopped backup, note the current source SHA,
and retain the current image under another tag, such as `teatro:before-upgrade`.
Review changes, check out the intended public revision, then:

```bash
docker compose build
docker compose up -d
docker compose logs --tail=100 teatro
curl --fail http://127.0.0.1:4440/healthz
```

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
