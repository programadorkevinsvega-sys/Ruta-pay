vault {
  address = "https://interserver-vault.tail4eafc5.ts.net:8190"
}

auto_auth {
  method "approle" {
    mount_path = "auth/approle"
    config = {
      role_id_file_path                   = "/etc/payvia/staging/vault-agent/role_id"
      secret_id_file_path                 = "/etc/payvia/staging/vault-agent/secret_id"
      remove_secret_id_file_after_reading = false
    }
  }
}

template_config {
  lease_renewal_threshold = 0.75
  exit_on_retry_failure   = true
}

template {
  source      = "/etc/payvia/staging/vault-agent/templates/nats-server.ctmpl"
  destination = "/etc/payvia/staging/vault-agent/rendered/nats-server.pem"
  perms       = "0600"
  backup      = false

  exec {
    command = [
      "/usr/local/libexec/payvia-install-pki-certificate.py",
      "/etc/payvia/staging/vault-agent/rendered/nats-server.pem",
      "/etc/payvia/staging/nats/tls/payvia/staging/server.crt",
      "/etc/payvia/staging/nats/tls/payvia/staging/server.key",
      "/etc/payvia/staging/nats/pending-tls/payvia/staging/ca.pem",
      "nats-stream.tail4eafc5.ts.net",
      "-",
      "server",
      "0",
      "0",
      "nats",
      "payvia-staging-nats-nats-1",
    ]
    timeout = "30s"
  }
}
