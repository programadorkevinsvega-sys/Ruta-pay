vault {
  address = "https://interserver-vault.tail4eafc5.ts.net:8190"
}

auto_auth {
  method "approle" {
    mount_path = "auth/approle"
    config = {
      role_id_file_path                   = "/run/payvia/staging/vault-credentials/role_id"
      secret_id_file_path                 = "/run/payvia/staging/vault-credentials/secret_id"
      remove_secret_id_file_after_reading = false
    }
  }

  sink "file" {
    config = {
      path = "/run/payvia/staging/vault-agent/token"
      mode = 0600
    }
  }
}
