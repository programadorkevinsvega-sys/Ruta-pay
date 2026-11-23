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
  source      = "/etc/payvia/staging/vault-agent/templates/postgres-server.ctmpl"
  destination = "/etc/payvia/staging/vault-agent/rendered/postgres-server.pem"
  perms       = "0600"
  backup      = false

  exec {
    command = [
      "/usr/local/libexec/payvia-install-pki-certificate.py",
      "/etc/payvia/staging/vault-agent/rendered/postgres-server.pem",
      "/etc/payvia/staging/postgres/tls/payvia/staging/server.crt",
      "/etc/payvia/staging/postgres/tls/payvia/staging/server.key",
      "/etc/payvia/staging/postgres/client-ca/ca.pem",
      "postgres.tail4eafc5.ts.net",
      "postgres",
      "server",
      "70",
      "70",
      "postgres",
      "payvia-staging-postgres-postgres-1",
    ]
    timeout = "30s"
  }
}

template {
  source      = "/etc/payvia/staging/vault-agent/templates/postgres-exporter.ctmpl"
  destination = "/etc/payvia/staging/vault-agent/rendered/postgres-exporter.pem"
  perms       = "0600"
  backup      = false

  exec {
    command = [
      "/usr/local/libexec/payvia-install-pki-certificate.py",
      "/etc/payvia/staging/vault-agent/rendered/postgres-exporter.pem",
      "/etc/payvia/staging/postgres/exporter-client/postgres-exporter.crt",
      "/etc/payvia/staging/postgres/exporter-client/postgres-exporter.key",
      "/etc/payvia/staging/postgres/server-ca/ca.pem",
      "postgres_exporter",
      "-",
      "client",
      "65534",
      "65534",
      "none",
      "-",
    ]
    timeout = "30s"
  }
}
