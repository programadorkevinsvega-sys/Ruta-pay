# Shared Vault

This component deploys one HashiCorp Vault server for Payvia and other applications. It uses Vault Integrated Storage (Raft) on a single VM, listens on that VM's Tailscale address and on loopback for local access, and exposes the UI and API on the same HTTPS port at each address. The Tailscale listener uses TCP/8190 (cluster TCP/8203); loopback uses TCP/8299 (cluster TCP/8205). This is a single-node deployment, not high availability. The TLS certificate is issued for the VM's Tailscale DNS name; local clients must use that hostname for certificate verification.

The addresses in `192.0.2.0/24` are documentation placeholders. The staging configuration uses the Vault VM's Tailscale address and DNS name; do not expose credentials or private keys.

## Security and operations

- Keep UFW default-deny for inbound traffic. Permit TCP/8190 only on `tailscale0` from the Tailnet; do not expose it on the public interface. Ports 8203, 8205, and 8299 are not opened for inbound traffic.
- The listener certificate is issued for the VM's Tailscale DNS name. Keep the private key readable only by the Vault container's numeric UID/GID `100:1000`.
- Raft data, local audit logs, PKI files, TLS material, snapshot credentials, runtime tokens, and snapshots live on the host under `/opt/payvia/staging/vault`; the main Vault container sees its data at `/opt/vault`. The Vault container runs with bounded CPU/memory, a read-only root filesystem, dropped capabilities, and core dumps disabled.
- Shamir initialization is 5 shares with a threshold of 3. Store shares separately outside the VM. Restarts require manual unseal.
- Snapshots are written locally every six hours and retained for 14 days. They do not protect against loss or compromise of this VM; test restoration on an isolated Vault instance before relying on recovery.
- The local file audit device is enabled. A protected remote audit receiver is not yet configured; local audit logs are not an off-host backup. Do not treat this deployment as meeting the remote-audit staging acceptance criterion until that receiver is configured and tested.
- The operator `userpass` policy can administer secrets for every application. Assign it only to trusted human operators. Workloads must use their own KV v2 mount, policy, and AppRole.

## Prepare the VM

Commands in this section run on the Vault VM over SSH as the administrative host user with `sudo`. Copy this directory to `/opt/payvia/staging/vault` first. The VM must already have Tailscale, Docker Compose, UFW, and the Vault TLS certificate configured.

Create persistent directories owned by the Vault container identity and install the server configuration:

```bash
sudo install -d -o 100 -g 1000 -m 0700 \
  /opt/payvia/staging/vault/data /opt/payvia/staging/vault/audit \
  /opt/payvia/staging/vault/pki /opt/payvia/staging/vault/snapshot-agent \
  /opt/payvia/staging/vault/snapshots /opt/payvia/staging/vault/tls \
  /opt/payvia/staging/vault/runtime
sudo chown "$(id -u):$(id -g)" /opt/payvia/staging/vault/.env
sudo chmod 0600 /opt/payvia/staging/vault/.env
sudo chmod 0644 /opt/payvia/staging/vault/vault.hcl \
  /opt/payvia/staging/vault/snapshot-agent.hcl
```

The server and snapshot Compose files read the same protected `.env` from `/opt/payvia/staging/vault`. Keep host data, TLS material, and snapshot-agent credentials under that staging directory; `/opt/vault` paths are only container-internal mount points. Validate the server Compose file before starting Vault:

The official Vault image entrypoint automatically loads configuration files from `/vault/config`. Do not pass an additional `-config` argument in the Compose command, or the same listener configuration can be loaded twice.

```bash
sudo docker compose --env-file /opt/payvia/staging/vault/.env \
  -f /opt/payvia/staging/vault/compose.yaml config --quiet
```

Allow only the Tailnet listener and start Vault:

```bash
sudo ufw allow in on tailscale0 from 100.64.0.0/10 \
  to 100.110.114.7 port 8190 proto tcp
sudo ufw status numbered
sudo docker compose --env-file /opt/payvia/staging/vault/.env \
  -f /opt/payvia/staging/vault/compose.yaml up -d
sudo docker exec shared-vault vault status
```

Before initialization, verify the listeners are bound to `100.110.114.7:8190` and `127.0.0.1:8299`, the TLS certificate matches the hostname, and Vault reports `Initialized false` and `Sealed true`. Initialize exactly once from a private interactive terminal:

```bash
sudo docker exec -it shared-vault \
  vault operator init -key-shares=5 -key-threshold=3
```

Store the five shares separately from the VM and each other; never paste shares or the initial root token into chat or logs. Unseal with three different shares, each in a separate invocation of `vault operator unseal`. Verify `Sealed false` and `HA Mode active` before continuing.

## Bootstrap and human access

The bootstrap runs inside the Vault container. Enter the initial root token only at the hidden prompt:

```bash
sudo docker exec -it shared-vault sh /vault/config/bootstrap.sh
```

It enables the local file audit device, `userpass`, AppRole, operator/snapshot policies, the snapshot AppRole, and the intermediate PKI mount. The generated intermediate CSR is stored on the host at `/opt/payvia/staging/vault/pki/intermediate.csr` for signing by an offline root CA. The script does not create a shared KV mount.

Create a named human account from a workstation connected to Tailscale with Vault CLI installed and TLS verification enabled:

```bash
export VAULT_ADDR=https://interserver-vault.tail4eafc5.ts.net:8190
./create-ui-user.sh
```

The script prompts for username, policy, administrator token, and password without echoing credentials. It can create or update the account. Sign in at `/ui` using the `Userpass` method and verify the account before retiring routine use of the root token. Never assign `vault-operator` to a user who should not administer all application secrets.

## Local Raft snapshots

After bootstrap, the snapshot AppRole credential files must exist under `/opt/payvia/staging/vault/snapshot-agent/`. Validate the combined Compose configuration and start the agent and snapshotter:

```bash
sudo docker compose --env-file /opt/payvia/staging/vault/.env \
  -f /opt/payvia/staging/vault/compose.yaml \
  -f /opt/payvia/staging/vault/compose.snapshots.yaml config --quiet
sudo docker compose --env-file /opt/payvia/staging/vault/.env \
  -f /opt/payvia/staging/vault/compose.yaml \
  -f /opt/payvia/staging/vault/compose.snapshots.yaml \
  up -d vault-snapshot-agent vault-snapshotter
```

Verify both containers are running, the Agent authenticates, and a non-empty `raft-*.snap` file appears in `/opt/payvia/staging/vault/snapshots`. The snapshotter retries failures and applies a 14-day local retention policy.

## Tailscale certificate renewal

On the Vault VM, install the renewal script and systemd units, run the service once, then enable the weekly timer:

```bash
sudo install -o root -g root -m 0755 \
  /opt/payvia/staging/vault/vault-tailscale-cert-renew.sh \
  /usr/local/sbin/vault-tailscale-cert-renew
sudo install -o root -g root -m 0644 \
  /opt/payvia/staging/vault/vault-tailscale-cert-renew.service \
  /etc/systemd/system/vault-tailscale-cert-renew.service
sudo install -o root -g root -m 0644 \
  /opt/payvia/staging/vault/vault-tailscale-cert-renew.timer \
  /etc/systemd/system/vault-tailscale-cert-renew.timer
sudo systemctl daemon-reload
sudo systemctl start vault-tailscale-cert-renew.service
sudo systemctl enable --now vault-tailscale-cert-renew.timer
```

Verify the service result, active timer, certificate hostname/expiry, and file ownership/modes. The script validates the new certificate and key match before installing them and signals the running Vault process to reload TLS material.

## Per-application isolation

From an authorized administrative workstation, provision one app at a time:

```bash
export VAULT_ADDR=https://interserver-vault.tail4eafc5.ts.net:8190
./provision-app.sh <app-slug> <secure-output-directory>
```

The output directory must already exist, be operator-owned, and protected with restrictive permissions. Provisioning creates a dedicated KV v2 mount, policy, and AppRole. Deliver the response-wrapped SecretID using an approved secure channel. Confirm application A cannot read or list application B's mount. Do not use shared credentials or a global KV mount.

## Service PKI and secret lifecycle

### Single-VM deployment scope

The current topology has one Vault VM and separate VMs for the application, PostgreSQL, Redis, and NATS services. This deployment intentionally runs one Vault Raft node; it does not create a replica pool or claim high availability. Adding Vault Raft peers is an optional future change requiring additional Vault VMs, a reviewed failure-domain design, and tested quorum/restore procedures. The current node remains the only online signing service, so its outage prevents new issuance and renewal; existing services continue only while their installed certificates and credentials remain valid.

The selected maximum leaf-certificate lifetime is 2160 hours (90 days) to provide recovery margin for the single Vault VM. Vault Agent renews at 75% of the certificate lifetime (67.5 days after issue, leaving 22.5 days before expiration); outage/recovery tests must verify that this margin is operationally adequate. IP SAN issuance is not permitted. Tailscale status and DNS resolution from the current provisioning workstation returned these exact service names:

| Service | Tailscale DNS identity | Container-local alias used by staging clients |
|---|---|---|
| PostgreSQL | `postgres.tail4eafc5.ts.net` | `postgres` |
| Redis | `redis.tail4eafc5.ts.net` | `redis` |
| NATS | `nats-stream.tail4eafc5.ts.net` | None for client TLS |
| Vault | `interserver-vault.tail4eafc5.ts.net` | Not applicable |

The three service names resolved to the existing Tailscale IPs (`100.86.36.108`, `100.88.251.53`, and `100.121.121.12`) from the provisioning workstation. On 2026-11-13, DNS resolution was confirmed from the application VM (`ubuntu@ip-10-0-2-70`) and the Vault VM (`vpsadmin@vps3812890`). The application VM is the direct client of PostgreSQL, Redis, and NATS. Vault is intended to store their credentials in KV and deliver them to workloads through its API/Agent after that integration is deployed; it does not need data-plane connections to those services. The DNS query from the Vault VM only confirmed name resolution and does not authorize adding Vault to service ACLs, `pg_hba.conf`, or firewall rules. All three names resolved to the expected IPs from the application VM, completing the relevant client DNS check. Server certificates must include the service FQDN and, for PostgreSQL and Redis only, their internal Compose alias as exact DNS SANs. Service restarts during rotation are not authorized: each service and client process must pass a hot-reload/reconnection acceptance before Vault Agent is wired to it.

The DNS check has been run from the direct data client, the application VM. For future verification, run the following on any VM that directly connects to these services, and verify each answer matches the current Tailscale address of the named service VM before changing the endpoint. These commands are read-only and do not alter connections:

```bash
getent ahostsv4 postgres.tail4eafc5.ts.net
getent ahostsv4 redis.tail4eafc5.ts.net
getent ahostsv4 nats-stream.tail4eafc5.ts.net
```

Record the VM, command output, and date in the staging deployment record. The result from the provisioning workstation is not a substitute for this client-side check. The application VM has passed; the Vault VM's DNS result is informational only because Vault does not connect to the data services.

The implementation sequence is deliberately gated:

1. Keep the staging root CA offline and separate from production. Copy `/opt/payvia/staging/vault/pki/intermediate.csr`, the public root certificate, and the root private key only to the controlled offline signing workstation. Use a private working directory (`chmod 700`) and root key mode `0600`; sign the CSR with [`sign-pki-intermediate-offline.sh`](../../vault/sign-pki-intermediate-offline.sh), which verifies the CSR subject/signature, root key match and CA constraints, limits the intermediate to `pathlen:0`, and refuses to overwrite an existing output. For example, after transferring the CSR and public root certificate through the approved channel, run `./sign-pki-intermediate-offline.sh ./intermediate.csr ./staging-root-ca.pem ./staging-root-ca-key.pem 1825`. Transfer only the resulting public `intermediate-signed.pem` back to the authorized Vault operator. Never connect the root-key workstation to Vault or copy the root private key to a VM, Vault, or this repository. The signer does not access Vault.
2. Use [`configure-pki-issuer.sh`](../../vault/configure-pki-issuer.sh) from an authorized workstation to verify and import the signed intermediate chain and set issuer/CRL URLs. On the first import, Vault may not yet have a default issuer; the script must allow that specific unconfigured state while refusing to replace an already configured issuer. Then use [`configure-pki-role.sh`](../../vault/configure-pki-role.sh) for each inventoried identity; roles are restricted to exact common name and explicitly listed DNS SANs, disallow IP SANs, subdomains and wildcards, use one key type/EKU, and cap leaf lifetime at 2160 hours. The scripts require the explicit `staging` environment and interactive confirmation of the exact Vault address. For example, provision PostgreSQL with `./configure-pki-role.sh staging postgres-server server postgres.tail4eafc5.ts.net postgres 2160`; Redis with `./configure-pki-role.sh staging redis-server server redis.tail4eafc5.ts.net redis 2160`; and NATS with `./configure-pki-role.sh staging nats-server server nats-stream.tail4eafc5.ts.net - 2160`. Client roles use the exact database/NATS authorization identity as common name; PostgreSQL maps the certificate CN to the database role. Use these FQDNs in client connection URLs; IP SANs and per-client IP allowlists are not used. Network reachability is restricted to the Tailnet/its ACL, while the certificate and database/ACL/subject permissions authorize each workload. The application VM's three DNS lookups passed on 2026-11-13. Vault's own DNS lookup is informational and does not make it a data-service client.
3. Use [`provision-pki-agent.sh`](./agent/provision-pki-agent.sh) to create one service-VM AppRole whose policy permits only the explicitly listed PKI issue roles for that VM and the public CA chain. The RoleID and single-use 15-minute response-wrapped SecretID are delivered only to that VM. [`install-agent-host.sh`](./agent/install-agent-host.sh) unwraps the reusable, 30-day SecretID on the destination, installs the pinned Vault binary/configuration, validates authentication, and enables the systemd Agent. Its templates render certificates at a 75% lifetime threshold; replacing the SecretID remains an operator-run action and must be completed before the 30-day expiry. Never copy the human bootstrap/root token or any root private key to a workload VM.
4. Validate the rendered certificate/key pair and chain before any service reload. PostgreSQL must apply its TLS-file update with `pg_ctl reload`/SIGHUP, Redis through its supported runtime TLS configuration update, and NATS through its documented SIGHUP reload; prove the service process did not restart, existing connections remain safe, and a new mTLS connection sees the new certificate. These checks must use the exact deployed versions and file paths.
5. Client processes are a separate blocking item. The Rust PostgreSQL pool, Redis connection manager, and NATS client are currently built at process startup; replacing Agent-rendered PEM files does not refresh their in-memory TLS configuration. They must support replacing/rebuilding pools or reconnecting with newly loaded TLS material without restarting the process, while draining old connections safely. No client-side hot reload is implemented or accepted yet.
6. Test issuance, renewal, expiry rejection, service reload, new authenticated connections, old-identity overlap, outage/recovery, and secret redaction against the actual isolated staging VMs before enabling the same process in production.

Vault's Raft cluster-address settings do not make a single node highly available. The 2160-hour leaf maximum and 75% renewal threshold are initial policy choices, not a load-tested availability guarantee. The exact DNS identities are identified above; verify them from all client VMs and prove each service-specific hot-reload procedure before issuing service identities.

The host Agent scripts install service-specific configuration and render/validate certificates; they do not prove that Redis or NATS has loaded a renewed certificate or that an application pool has reloaded a client identity. Do not treat file rendering as service acceptance. The separate [`provision-pki-workload.sh`](../../vault/provision-pki-workload.sh), [`unwrap-pki-secret-id.sh`](../../vault/unwrap-pki-secret-id.sh), and [`rotate-pki-workload-secret-id.sh`](../../vault/rotate-pki-workload-secret-id.sh) helpers remain available for other individual workloads.

Staging must use its own offline root CA, separate from production. The root private key stays offline and outside this repository; only its public trust chain is distributed to the staging consumers. The Vault Tailscale HTTPS certificate remains a separate transport identity and is not the issuer for PostgreSQL, Redis, NATS, applications, or exporters.

**Read-only operational check (2026-11-13):** `vault status` at the staging endpoint reported Vault initialized and unsealed (version 2.2.1). `vault list pki_int/roles` returned `nats-server`, `postgres-server`, `redis-server`, `postgres-exporter-client`, `redis-exporter-client`, and `redis-healthcheck-client`. This confirms the PKI mount and roles exist on the running instance, but does not prove service-side certificate renewal, client connection acceptance, or application identity provisioning.

There is no dedicated Phase 4 acceptance client role. Do not reuse exporter, healthcheck, or server identities for acceptance; their permissions and purposes differ. Provision a separate short-lived client identity only after its exact CN/DNS SAN, PostgreSQL role permissions, Redis ACL, NATS subjects/stream operations, and removal procedure have been reviewed and authorized. The earlier deployment/bootstrap prerequisite text no longer describes the current Vault instance. Staging's service FQDNs are identified; DNS resolution from the direct data client (the application VM) is verified, but that does not make this development host an authenticated data-service client. Vault's own DNS check is not a data-plane connectivity test. IP SAN issuance is deliberately disabled in the shared role helper because it cannot enforce an exact IP allowlist. No CSR signature, root key, service key, or live credential is committed here. Leaf-certificate lifetime and renewal threshold must be set explicitly in the roles and Agent templates; they must not inherit an unreviewed Vault default.

#### Phase 4 acceptance credentials

Keep acceptance credentials separate from `apps/platform` and application runtime identities. The local PostgreSQL acceptance runner requires its own isolated KV v2 mount and read-only AppRole; do not reuse `app-platform`, which can read and modify the application's KV entries. Vault Agent may render the PostgreSQL admin/runtime URLs and their short-lived mTLS material only into a private, untracked local session directory with mode `0600`. The runner consumes those rendered variables directly; it must not copy them into either backend `.env` or the application VM. The acceptance role must be short-lived, limited to the exact PostgreSQL test secret path, and revoked after the acceptance window. Redis and NATS credentials remain blocked until their distinct acceptance ACL/subject permissions are configured and authorized.

Each VM workload receives its own AppRole and single-use response-wrapped SecretID over an approved secure channel. A Vault Agent on the owning VM authenticates with that identity, renews its own token, and renders only the files needed by that workload with restrictive ownership and permissions. Services reload only through a fixed, reviewed command after the certificate/key pair and chain have been validated. Application secrets are stored per application in KV v2 and delivered as files; non-sensitive settings stay in Compose/`.env`. A process that only reads secrets from startup environment variables still needs a file-based integration or a verified restart/rollout procedure before its rotation is automated.

| Consumer | Vault-managed material | Required integration and acceptance |
|---|---|---|
| PostgreSQL server and clients | Staging Compose pins `postgres:18.6-alpine`; server files are `/run/tls/payvia/staging/server.crt`, `/run/tls/payvia/staging/server.key`, and `/run/tls/payvia/staging/client-ca/ca.pem`. | Preserve `hostssl ... cert` role identity. `pg_reload_conf()`/SIGHUP applies TLS file changes without restarting PostgreSQL; prove the PID/start time is unchanged and a new mTLS connection sees the new certificate. Missing/expired/untrusted certificates or wrong CN/SAN must fail. |
| Redis server and clients | Staging Compose pins `redis:8.10.2-alpine`; server files are `/run/payvia/staging/redis/tls/server.crt` and `/run/payvia/staging/redis/tls/server.key`; client CA is `/run/payvia/staging/redis/client-ca/ca.pem`. | Preserve `tls-auth-clients yes`. The pinned Redis build must prove a runtime TLS configuration update loads the rotated files, keeps the process PID unchanged, and allows a new authenticated connection. The official Vault Redis database plugin API documents TLS enablement and server verification, but no client certificate/key fields; do not lower Redis mTLS to make that plugin connect. |
| NATS server and clients | Staging Compose pins `nats:2.15.1-alpine`; server files are `/run/tls/payvia/staging/server.crt`, `/run/tls/payvia/staging/server.key`, and `/run/tls/payvia/staging/client-ca.pem`. | `verify_and_map` maps client certificate identity to NATS permissions. Use the documented SIGHUP TLS reload; prove the PID is unchanged, existing connections remain safe, and new mTLS connections see the rotated certificate. Only authorized certificate identities and subjects may work. |
| Backend and observability workloads | Workload-specific secret values in KV v2, including sensitive metrics credentials where applicable. | Render files with least privilege; verify application A cannot read application B's secret and that rotation reaches the process without exposing values in logs or Git. |

No certificate rotation is accepted merely because a new file was rendered: the service must load it successfully, new connections must use it, established connections must drain safely, and the previous identity must remain trusted until consumers have transitioned. Required integration and service restart/reconnection tests must run against isolated staging services before this lifecycle is enabled. See the [shared infrastructure matrix](../../README.md#matriz-de-gesti%C3%B3n-vault-por-servicio).

The current infrastructure has no product frontend, REST/BFF, gRPC, PostgreSQL schema, Redis cache, NATS event, or Outbox mutation in this Vault lifecycle change. The Vault API is an operator/workload control plane, not a user request path. Realtime/JetStream product events are therefore not applicable: issuance and renewal change machine credentials, not persisted user-visible Payvia business state. PostgreSQL, Redis, and NATS remain the owners of their service data and continue to be reached only by their authorized clients.

## Staging mTLS acceptance without restarts

### Vault Agent renewal design

The deployment uses a host-level Vault Agent as root under systemd on each data-service VM, with a dedicated AppRole scoped to that VM's PKI issue paths. The Redis Agent additionally has read-only access to the single KV v2 path `apps/platform/data/redis`; it renders the `platform-runtime` password to a root-only file consumed by the application delivery process. The Agent verifies Vault's HTTPS certificate using the operating-system trust store, keeps its unwrapped SecretID in a root-only file, and renders `pkiCert` bundles into a root-only persistent cache. PKI templates set `lease_renewal_threshold = 0.75`, so certificates are fetched/rotated before expiry; installation validates the CN, required DNS SANs, EKU, CA chain, and public/private key match before atomically replacing host files. The staging root private key is never delivered to Vault Agents.

The versioned files are in [`agent/`](./agent/). Provision one host-scoped AppRole from an authenticated staging operator workstation with `agent/provision-pki-agent.sh staging <postgres|redis|nats> <exact comma-separated roles>`. For Redis, create its dedicated rotation credential in Vault with `./provision-redis-tls-rotator.sh staging` before installing the updated Agent template. Transfer the resulting RoleID and short-lived response-wrapped SecretID only to that service VM along with the pinned Vault Agent binary and this directory. Run `sudo ./install-agent-host.sh <service>` there. The installer unwraps the SecretID on the destination, validates configuration/authentication, and enables the Agent systemd unit; it does not start or restart a data-service container. Redis host preparation must run after Vault Agent has rendered its runtime and rotator credentials and before Redis starts. Keep the 30-day SecretID in the protected destination file and rotate it manually before expiry; do not copy the human/root Vault token to a workload VM.

Vault Agent's `pkiCert` templates issue certificates with a 90-day lifetime and re-render at the configured 75% renewal threshold. Each render must validate the chain, identity, EKU, SANs, and key match, atomically install the new pair at the active bind-mounted paths, then activate it in-process: PostgreSQL receives `pg_ctl reload`, Redis receives an authenticated `CONFIG SET tls-cert-file ... tls-key-file ...`, and NATS receives SIGHUP. The installer fails visibly if validation or an attempted reload fails; it never restarts a data-service container. NATS must use the active `tls/` mount, not `pending-tls/`.

Redis's ACL must have a dedicated `payvia_tls_rotator` user limited to `PING` and `CONFIG SET`, separate from application, exporter, and healthcheck identities. Its password is held in Vault KV and rendered only to the Redis host with container-readable restrictive permissions; the active ACL stores only its hash. A root-owned marker prevents the Agent from reporting a successful TLS reload until this ACL is active. On an already-running Redis VM with no such identity, initial activation requires a one-time controlled Redis restart after installing the ACL entry; then remove the marker and invoke the installer once to verify in-process reload. On a clean VM, Redis loads the ACL during its first start, after which the marker can be removed without an additional restart. This bootstrap is distinct from certificate renewal; renewals thereafter reload Redis in-process without restarting it. The one-time bootstrap on staging Redis was explicitly approved and completed on 2026-11-14; the persistent `redis_data` volume was retained. The marker was removed after Redis became healthy, and a certificate reload then completed with the container start timestamp unchanged. Future bootstrap restarts still require an approved maintenance window.

The PostgreSQL/Redis exporter client certificates and Redis healthcheck certificate are also renewed by Vault Agent. Healthchecks start a fresh TLS client for each invocation. Exporter processes may retain client TLS configuration loaded at startup, so their client-certificate renewal must use a supported reload mechanism or a scoped exporter-only restart; this must not restart PostgreSQL or Redis and must be recorded separately. Redis application-password rotation is distinct from certificate rotation: changing the KV value does not update Redis's ACL hash or reload an application pool automatically. The application VM still needs its own AppRole delivery, client certificate, and a tested mechanism to reload its Rust pools; those application-host changes are not part of this deployment.

The versioned staging Compose/configuration files require mTLS on PostgreSQL, Redis, and NATS. Vault is initialized and unsealed, its staging intermediate has been signed offline and imported, issuer/CRL URLs are configured, and restricted PKI roles exist for PostgreSQL, Redis, and NATS. PostgreSQL server/exporter and Redis server/healthcheck/exporter certificates have been issued and installed at their configured host paths. NATS startup failed because `max_outstanding_pings` is not a NATS 2.15 server option and the issued server certificate was still staged outside the active TLS mount. The config now uses supported `ping_max`, and the verified Vault-issued server certificate is installed in the active TLS mount. NATS and its exporter are healthy; the JetStream volume was preserved. The server restart count reflects the prior failed-start loop and has remained stable since recovery.

| Service VM | Server identity | Local client identities | Host certificate paths |
|---|---|---|---|
| PostgreSQL (`100.86.36.108`) | `postgres.tail4eafc5.ts.net`, DNS SAN `postgres` | `postgres_exporter` | `/etc/payvia/staging/postgres/tls/payvia/staging/`, `client-ca/`, `server-ca/`, `exporter-client/` |
| Redis (`100.88.251.53`) | `redis.tail4eafc5.ts.net`, DNS SAN `redis` | `payvia_healthcheck`, `payvia_staging_exporter`, `payvia_tls_rotator` | `/etc/payvia/staging/redis/tls/`, `client-ca/`, `server-ca/`, `healthcheck-client/`, `exporter-client/` |
| NATS (`100.121.121.12`) | `nats-stream.tail4eafc5.ts.net` | Runtime client identities are installed on the application VM, not the NATS server VM | `/etc/payvia/staging/nats/tls/payvia/staging/` (active; Agent writes directly to this mount) |

For each issued leaf, transfer the private key directly over SSH to its owning VM, never commit it or put it in a shared delivery directory, verify certificate/key match and chain on the destination, and apply owner/mode for the exact container UID. Install the CA chain separately as required by the service Compose mounts. Issuance and file installation alone do not constitute an accepted TLS handshake.

The application VM is the sole direct client of these data services. Validate positive and negative client-certificate cases there using the actual application identities and service endpoints, without printing passwords, private keys, full connection URLs, or certificate contents. For each service, record the accepted TLS version and peer identity, and confirm missing/untrusted/expired client certificates and unauthorized roles/ACLs/subjects are rejected. A DNS lookup or a service healthcheck is not a substitute for an authenticated application handshake.

For server-certificate renewal, the Agent hook installs only a validated certificate/key pair and uses the service's in-process reload (PostgreSQL reload, Redis `CONFIG SET`, NATS SIGHUP). It must never restart the data-service container. Verify this by recording its start timestamp before and after the renewal and opening a new authenticated TLS connection with an authorized client. For example, from each service's Compose directory, the following read-only command records the running container timestamp (substitute `postgres`, `redis`, or `nats` for the service name):

```bash
docker compose --env-file .env -f compose.yaml ps -q postgres |
  xargs -r docker inspect --format '{{.Name}} {{.State.StartedAt}}'
```

Do not rotate or revoke the application client certificate until the application process can reload its TLS identity and replace/drain connections without restarting. The Redis password is now in Vault KV and is rendered on the Redis VM; the application VM's AppRole, client-certificate installation, startup configuration, and pool reload have not been deployed or tested.

## Current acceptance status

Staging Vault is initialized, unsealed, and active. Its offline-root-signed intermediate is imported, issuer/CRL URLs are configured, and restricted PKI roles exist for PostgreSQL, Redis, and NATS server identities plus PostgreSQL exporter and Redis healthcheck/exporter identities. Vault Agent v2.2.1 is active under systemd on all three service VMs with host-scoped AppRoles, root-only reusable SecretIDs, persistent private template caches, and 75%-lifetime PKI templates. PostgreSQL server/exporter and Redis server/healthcheck/exporter certificates were rendered by those Agents and validated/installed. The PostgreSQL and NATS hot-reload hooks were manually exercised against their currently rendered Vault-issued certificates. Redis's dedicated rotator ACL was activated during the approved one-time bootstrap, and its certificate hook was exercised afterward. These checks validate in-process activation, not fresh certificate issuance or the future 75%-lifetime renewal event; the 90-day renewal cycle has not elapsed. The Vault Tailscale certificate-renewal timer is enabled and active; its last trigger was 2026-11-09 with a successful unit result, and the Vault container is running without restarts. The application VM remains out of scope and unchanged.

On 2026-11-13, PostgreSQL startup was repaired by replacing the empty host directory at the `pg_hba.conf` bind source with the intended file and recreating only the container; its PostgreSQL 18 volume was preserved. The configured database and exporter role were created in-place because initialization had stopped before those bootstrap steps. PostgreSQL is healthy with zero restarts; its exporter reports `pg_up=1` and `pg_exporter_last_scrape_error=0`, confirming the mTLS scrape. The application VM still attempts PostgreSQL connections from private address `10.0.2.70`, which the TLS-only `pg_hba.conf` rejects; do not broaden access without an explicit network/authentication review.

Redis was started on its own VM with `maxmemory=512mb`, a 768 MiB container limit, and 0.8 CPU; its exporter is limited to 128 MiB and 0.2 CPU. Its persistent volume was empty before startup. The authenticated TLS healthcheck returns `PONG`, and the exporter reports `redis_up=1` with `redis_exporter_last_scrape_error=0`. A new restrictive ACL is active. The `platform-runtime` password is in KV v2 at `apps/platform/data/redis`; AppRole `app-platform` was tested to read only that entry and cannot list unrelated secrets or write data. Redis Agent also has read access to that single entry and renders the password at `/etc/payvia/staging/redis/application-credentials/platform-runtime-password`; the host file matched Vault and the password authenticated against the active Redis ACL over mTLS. The old plaintext passwords and backup ACL copies were removed from the Redis host. A separate `payvia_tls_rotator` ACL user is sourced from KV v2 and has only `PING` and `CONFIG SET`; its ACL stores the password hash, not the plaintext. After its authorized bootstrap, the Redis server certificate hook authenticated as this user, applied `CONFIG SET`, and passed a fresh TLS healthcheck without changing the container start timestamp. The container is healthy with restart count 0 since that bootstrap, and its `redis_data` volume remains mounted. One-use wrapped AppRole delivery material is held in the local protected operator directory `~/.payvia-vault-delivery/`; the application VM remains untouched, so actual application client-certificate mTLS and client-pool integration are pending.

The NATS Agent issued a server certificate for `nats-stream.tail4eafc5.ts.net`; its SAN, private-key match, and chain to the offline root were verified before installation. On 2026-11-13, startup was repaired by replacing unsupported `max_outstanding_pings` with the NATS 2.15 `ping_max` setting and installing the validated certificate/key/client CA in the active mount. NATS reports `Server is ready`, its `/healthz` returns `ok`, and the exporter is running and exposing metrics; the JetStream volume was preserved. The server's restart count of 225 is from the prior failed-start loop and remained unchanged after recovery and an in-process SIGHUP certificate reload. PostgreSQL's `pg_ctl reload` hook was also exercised successfully; its container remained healthy with zero restarts and an unchanged start timestamp. These PostgreSQL, Redis, and NATS reload checks used certificates already rendered by Vault Agent and do not prove a new certificate was issued by the automatic renewal cycle. An application-client mTLS handshake and authorization test is still pending because the application VM was not changed; a raw `openssl s_client` probe was not a valid NATS client handshake and was inconclusive. Client-pool reload, exporter client-certificate reload, remote audit ingestion, and isolated snapshot restore remain unverified. DNS resolution from the application VM, the direct data client, is verified; the Vault VM's DNS result does not imply or require data-plane connectivity. Local snapshots do not provide disaster recovery if the VM is lost.
