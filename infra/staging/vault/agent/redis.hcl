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
  source      = "/etc/payvia/staging/vault-agent/templates/redis-server.ctmpl"
  destination = "/etc/payvia/staging/vault-agent/rendered/redis-server.pem"
  perms       = "0600"
  backup      = false

  exec {
    command = [
      "/usr/local/libexec/payvia-install-pki-certificate.py",
      "/etc/payvia/staging/vault-agent/rendered/redis-server.pem",
      "/etc/payvia/staging/redis/tls/server.crt",
      "/etc/payvia/staging/redis/tls/server.key",
      "/etc/payvia/staging/redis/client-ca/ca.pem",
      "redis.tail4eafc5.ts.net",
      "redis",
      "server",
      "999",
      "1000",
      "redis",
      "payvia-staging-redis-redis-1",
    ]
    timeout = "30s"
  }
}

template {
  source      = "/etc/payvia/staging/vault-agent/templates/redis-healthcheck.ctmpl"
  destination = "/etc/payvia/staging/vault-agent/rendered/redis-healthcheck.pem"
  perms       = "0600"
  backup      = false

  exec {
    command = [
      "/usr/local/libexec/payvia-install-pki-certificate.py",
      "/etc/payvia/staging/vault-agent/rendered/redis-healthcheck.pem",
      "/etc/payvia/staging/redis/healthcheck-client/redis-healthcheck.crt",
      "/etc/payvia/staging/redis/healthcheck-client/redis-healthcheck.key",
      "/etc/payvia/staging/redis/server-ca/ca.pem",
      "payvia_healthcheck",
      "-",
      "client",
      "999",
      "1000",
      "none",
      "-",
    ]
    timeout = "30s"
  }
}

template {
  source      = "/etc/payvia/staging/vault-agent/templates/redis-exporter.ctmpl"
  destination = "/etc/payvia/staging/vault-agent/rendered/redis-exporter.pem"
  perms       = "0600"
  backup      = false

  exec {
    command = [
      "/usr/local/libexec/payvia-install-pki-certificate.py",
      "/etc/payvia/staging/vault-agent/rendered/redis-exporter.pem",
      "/etc/payvia/staging/redis/exporter-client/payvia_staging_exporter.crt",
      "/etc/payvia/staging/redis/exporter-client/payvia_staging_exporter.key",
      "/etc/payvia/staging/redis/server-ca/ca.pem",
      "payvia_staging_exporter",
      "-",
      "client",
      "59000",
      "59000",
      "none",
      "-",
    ]
    timeout = "30s"
  }
}

template {
  source      = "/etc/payvia/staging/vault-agent/templates/redis-platform-runtime.ctmpl"
  destination = "/etc/payvia/staging/redis/application-credentials/platform-runtime-password"
  perms       = "0400"
  backup      = false
}

template {
  source      = "/etc/payvia/staging/vault-agent/templates/redis-tls-rotator.ctmpl"
  destination = "/etc/payvia/staging/vault-agent/rendered/redis-tls-rotator.password"
  perms       = "0600"
  backup      = false

  exec {
    command = [
      "/usr/local/libexec/payvia-install-redis-tls-rotator.py",
      "/etc/payvia/staging/vault-agent/rendered/redis-tls-rotator.password",
      "/etc/payvia/staging/redis/rotation-credentials/tls-rotator-password",
      "/etc/payvia/staging/redis/acl/users.acl",
    ]
    timeout = "30s"
  }
}
