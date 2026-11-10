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

## Current acceptance status

The deployed single-node instance has been initialized and unsealed, the UI operator login was verified, local audit is enabled, a non-empty local snapshot was observed, and weekly certificate renewal completed successfully. Remote audit ingestion and isolated snapshot restore have not been verified. Local snapshots do not provide disaster recovery if the VM is lost.
