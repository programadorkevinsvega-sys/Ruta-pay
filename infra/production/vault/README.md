# Shared Vault

This component deploys one HashiCorp Vault server for Payvia and other applications. It uses Vault Integrated Storage (Raft) on a single VM, listens only on that VM's Tailscale address, and exposes the UI and API on the same HTTPS port. This is a single-node deployment, not high availability.

Addresses in `192.0.2.0/24` and `vault.example.invalid` are documentation placeholders. Replace them with the deployment's private addresses and Tailscale DNS name in the private copy before running any commands; do not commit those live values.

## Security and operations

- Keep UFW default-deny for inbound traffic. Permit TCP/8200 only on `tailscale0` from the Tailnet; do not expose it on the public interface. Port 8201 is not opened for this single-node setup.
- The listener certificate is issued for the VM's Tailscale DNS name. Keep the private key readable only by the Vault container's numeric UID/GID `100:1000`.
- Raft data, local audit logs, PKI files, and snapshots live under `/opt/vault`. The Vault container runs with bounded CPU/memory, a read-only root filesystem, dropped capabilities, and core dumps disabled.
- Shamir initialization is 5 shares with a threshold of 3. Store shares separately outside the VM. Restarts require manual unseal.
- Snapshots are written locally every six hours and retained for 14 days. They do not protect against loss or compromise of this VM; test restoration on an isolated Vault instance before relying on recovery.
- The local file audit device is enabled. A protected remote audit receiver is not yet configured; local audit logs are not an off-host backup. Do not treat this deployment as meeting the remote-audit production acceptance criterion until that receiver is configured and tested.
- The operator `userpass` policy can administer secrets for every application. Assign it only to trusted human operators. Workloads must use their own KV v2 mount, policy, and AppRole.

## Prepare the VM

Commands in this section run on the Vault VM over SSH as the administrative host user with `sudo`. Copy this directory to `~/infra/production/vault` first. The VM must already have Tailscale, Docker Compose, UFW, and the Vault TLS certificate configured.

Create persistent directories owned by the Vault container identity and install the server configuration:

```bash
sudo install -d -o 100 -g 1000 -m 0700 \
  /opt/vault/data /opt/vault/audit /opt/vault/pki \
  /opt/vault/snapshot-agent /opt/vault/snapshots /opt/vault/tls
sudo install -d -o root -g root -m 0755 /etc/vault/config
sudo install -o root -g root -m 0644 \
  ~/infra/production/vault/vault.hcl /etc/vault/config/vault.hcl
```

Install `/etc/vault/vault.env` using the values in `.env.example`, mode `0600`. Validate the server Compose file before starting Vault:

```bash
sudo docker compose --env-file /etc/vault/vault.env \
  -f ~/infra/production/vault/compose.yaml config --quiet
```

Allow only the Tailnet listener and start Vault:

```bash
sudo ufw allow in on tailscale0 from 192.0.2.0/10 \
  to 100.110.114.7 port 8200 proto tcp
sudo ufw status numbered
sudo docker compose --env-file /etc/vault/vault.env \
  -f ~/infra/production/vault/compose.yaml up -d
sudo docker exec shared-vault vault status
```

Before initialization, verify the listener is bound only to `100.110.114.7:8200`, the TLS certificate matches the hostname, and Vault reports `Initialized false` and `Sealed true`. Initialize exactly once from a private interactive terminal:

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

It enables the local file audit device, `userpass`, AppRole, operator/snapshot policies, the snapshot AppRole, and the intermediate PKI mount. The generated intermediate CSR is stored in `/opt/vault/pki/intermediate.csr` for signing by an offline root CA. The script does not create a shared KV mount.

Create a named human account from a workstation connected to Tailscale with Vault CLI installed and TLS verification enabled:

```bash
export VAULT_ADDR=https://vault.example.invalid:8200
./create-ui-user.sh
```

The script prompts for username, policy, administrator token, and password without echoing credentials. It can create or update the account. Sign in at `/ui` using the `Userpass` method and verify the account before retiring routine use of the root token. Never assign `vault-operator` to a user who should not administer all application secrets.

## Local Raft snapshots

After bootstrap, the snapshot AppRole credential files must exist under `/opt/vault/snapshot-agent/`. Validate the combined Compose configuration and start the agent and snapshotter:

```bash
sudo docker compose --env-file /etc/vault/vault.env \
  -f ~/infra/production/vault/compose.yaml \
  -f ~/infra/production/vault/compose.snapshots.yaml config --quiet
sudo docker compose --env-file /etc/vault/vault.env \
  -f ~/infra/production/vault/compose.yaml \
  -f ~/infra/production/vault/compose.snapshots.yaml \
  up -d vault-snapshot-agent vault-snapshotter
```

Verify both containers are running, the Agent authenticates, and a non-empty `raft-*.snap` file appears in `/opt/vault/snapshots`. The snapshotter retries failures and applies a 14-day local retention policy.

## Tailscale certificate renewal

On the Vault VM, install the renewal script and systemd units, run the service once, then enable the weekly timer:

```bash
sudo install -o root -g root -m 0755 \
  ~/infra/production/vault/vault-tailscale-cert-renew.sh \
  /usr/local/sbin/vault-tailscale-cert-renew
sudo install -o root -g root -m 0644 \
  ~/infra/production/vault/vault-tailscale-cert-renew.service \
  /etc/systemd/system/vault-tailscale-cert-renew.service
sudo install -o root -g root -m 0644 \
  ~/infra/production/vault/vault-tailscale-cert-renew.timer \
  /etc/systemd/system/vault-tailscale-cert-renew.timer
sudo systemctl daemon-reload
sudo systemctl start vault-tailscale-cert-renew.service
sudo systemctl enable --now vault-tailscale-cert-renew.timer
```

Verify the service result, active timer, certificate hostname/expiry, and file ownership/modes. The script validates the new certificate and key match before installing them and signals the running Vault process to reload TLS material.

## Per-application isolation

From an authorized administrative workstation, provision one app at a time:

```bash
export VAULT_ADDR=https://vault.example.invalid:8200
./provision-app.sh <app-slug> <secure-output-directory>
```

The output directory must already exist, be operator-owned, and protected with restrictive permissions. Provisioning creates a dedicated KV v2 mount, policy, and AppRole. Deliver the response-wrapped SecretID using an approved secure channel. Confirm application A cannot read or list application B's mount. Do not use shared credentials or a global KV mount.

## Service PKI and secret lifecycle

### Single-VM deployment scope

The selected initial production topology has one Vault VM and separate VMs for the application, PostgreSQL, Redis, and NATS services. It intentionally runs one Vault Raft node and no replica pool. Adding peers is an optional future architecture change and requires additional Vault VMs, a reviewed failure-domain design, and tested quorum, backup, and restore procedures; it is not enabled by these files. A single Vault node is a single point of failure: its outage prevents new issuance and renewal, and workloads can continue only until their installed credentials expire.

Production onboarding is gated on all of the following: replacing documentation-only network values with verified private/Tailscale addresses; an independently controlled offline production root; its signed intermediate chain; exact production service DNS identities; the selected 2160-hour maximum leaf lifetime and an explicit renewal threshold; separate workload identities; and real acceptance of reload, new connections, overlap, outage recovery, and redaction. The shared PKI-role helper intentionally disallows IP SANs because Vault's role-wide IP SAN switch does not provide an exact IP allowlist. Service clients must therefore use stable private DNS names matching their certificate SANs unless a separately reviewed issuance control is implemented. Service restarts during rotation are not authorized; all service and client processes must pass hot-reload/reconnection acceptance first. No production leaf certificate or credential should be created before those prerequisites are met. The shared [`configure-pki-issuer.sh`](../../vault/configure-pki-issuer.sh), [`configure-pki-role.sh`](../../vault/configure-pki-role.sh), and [`provision-pki-workload.sh`](../../vault/provision-pki-workload.sh) scripts require the explicit environment and an interactive confirmation of the exact Vault address.

The intended operational flow is: sign and verify the Vault-generated CSR offline; import the signed intermediate and configure scoped PKI roles; provision one AppRole and policy per workload; deliver the single-use, 15-minute response-wrapped token through an approved channel; unwrap it on the owning VM into a protected reusable SecretID with a 30-day TTL; run a local Agent to render protected files; validate the new material; then reload or deliberately restart only the owning service using a reviewed procedure. [`rotate-pki-workload-secret-id.sh`](../../vault/rotate-pki-workload-secret-id.sh) creates replacement credentials, but an operator must install them, restart/authenticate the Agent, and verify success before the old credential expires. This is not scheduled or unattended. The shared scripts do not install Agents, distribute credentials, rotate service files, or prove reload/reconnection. Secrets and certificates remain off Git. The Vault Raft cluster-address setting does not imply high availability.

Production must use its own offline root CA, cryptographically separate from staging. The root private key stays offline and outside this repository; only the production public trust chain is distributed to production consumers. The Vault Tailscale HTTPS certificate is a separate transport identity and must not issue service certificates.

The `pki_int/` mount must contain an intermediate signed by the production-only root before any production identity is issued. Configure narrow PKI roles using actual production DNS identities, workload identities, usages, and bounded lifetimes; values such as `192.0.2.0/24` and `vault.example.invalid` in this repository are documentation placeholders, not deployable SANs or endpoints. The shared helper disables IP SANs because Vault's role-wide `allow_ip_sans` option is not an exact IP allowlist; use stable private DNS names or implement and review a separate issuance control before enabling certificate issuance for IP-based connections. No root key, CSR signature, service key, or live credential belongs in Git.

Give each service or exporter VM its own AppRole and single-use response-wrapped SecretID through an approved secure channel. A local Vault Agent renews its token and renders only the needed certificate/key/chain and KV v2 secrets with restrictive ownership and permissions. Each consumer needs a fixed, reviewed reload or rolling-restart procedure, tested with real new connections before old identities are revoked. Non-sensitive configuration stays outside Vault. Applications that only consume startup environment variables require a file-based integration or verified restart/rollout before secret rotation can be considered automatic.

| Consumer | Vault-managed material | Required integration and acceptance |
|---|---|---|
| PostgreSQL server and clients | Server, runtime, migration, and exporter certificates; any required bootstrap/admin secrets in the PostgreSQL-owned KV path. | Preserve `hostssl ... cert` role identity. Prove valid mTLS succeeds and missing/expired/untrusted certificates or wrong CN/SAN fail; reload the server and establish new connections after rotation. |
| Redis server and clients | Server/client certificates and distinct ACL identities; ACL passwords remain in a per-workload KV path until a compatible mTLS-aware rotation controller exists. | Preserve `tls-auth-clients yes`. The official Vault Redis database plugin API documents TLS enablement and server verification, but no client certificate/key fields. Do not lower Redis mTLS to make that plugin connect. Prove any future ACL rotation keeps an overlap window, updates consumers, then revokes the old identity. |
| NATS server and clients | Server/client certificates and per-workload identity; `verify_and_map` maps client certificate identity to NATS permissions. | Prove only authorized certificate identities and subjects work. Validate the server reload and new client connections after rotation; keep permission changes separate from secret delivery. |
| Backend and observability workloads | Workload-specific secret values in KV v2, including sensitive metrics credentials where applicable. | Render files with least privilege; verify application A cannot read application B's secret and that rotation reaches the process without exposing values in logs or Git. |

Vault's [PostgreSQL database plugin](https://developer.hashicorp.com/vault/api-docs/secret/databases/postgresql) documents client certificate/key parameters, while the [Redis plugin API](https://developer.hashicorp.com/vault/api-docs/secret/databases/redis) does not. Do not disable required mTLS as a workaround. Vault Agent [templates](https://developer.hashicorp.com/vault/docs/agent-and-proxy/agent/template) can refresh files, but acceptance also requires proving that the receiving process loads them and that old connections/identities drain safely.

This infrastructure lifecycle has no product frontend, REST/BFF, gRPC request, module schema, Redis cache, NATS event, or Outbox mutation. Vault is a workload/operator control plane and must not be queried once per end-user operation. Realtime/JetStream product events are not applicable because rotating machine credentials does not mutate user-visible Payvia business state. Data ownership and service authorization remain with PostgreSQL, Redis, NATS, and their application clients.

## Current acceptance status

This directory is an operational template with placeholder addresses; those values must not be used for production. The single-node Vault design has no HA, remote audit ingestion and isolated snapshot restore have not been verified, and this repository does not establish that a production intermediate has been signed/imported or that production workloads use Vault Agent. The Tailscale listener-certificate renewal is separate from the service PKI lifecycle. Production secret/certificate automation is not accepted until the production network inventory, offline root, signed intermediate, scoped roles, workload identities, service reloads, and real integration/rotation tests are verified.
