vault {
  address = "https://vault.example.invalid:8200"
}

auto_auth {
  method "approle" {
    mount_path = "auth/approle"
    config = {
      role_id_file_path                   = "/run/vault-credentials/role_id"
      secret_id_file_path                 = "/run/vault-credentials/secret_id"
      remove_secret_id_file_after_reading = false
    }
  }

  sink "file" {
    config = {
      path = "/run/vault-agent/token"
      mode = 0600
    }
  }
}
