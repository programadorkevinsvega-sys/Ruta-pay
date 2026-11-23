ui = true
disable_mlock = true

api_addr = "https://interserver-vault.tail4eafc5.ts.net:8190"
cluster_addr = "https://interserver-vault.tail4eafc5.ts.net:8203"
cluster_name = "shared-vault"

storage "raft" {
  path    = "/opt/vault/data"
  node_id = "shared-vault-1"
}

listener "tcp" {
  address         = "100.110.114.7:8190"
  cluster_address = "100.110.114.7:8203"
  tls_cert_file   = "/opt/vault/tls/tailscale.crt"
  tls_key_file    = "/opt/vault/tls/tailscale.key"
  tls_min_version = "tls12"
  tls_max_version = "tls13"
}

listener "tcp" {
  address         = "127.0.0.1:8299"
  cluster_address = "127.0.0.1:8205"
  tls_cert_file   = "/opt/vault/tls/tailscale.crt"
  tls_key_file    = "/opt/vault/tls/tailscale.key"
  tls_min_version = "tls12"
  tls_max_version = "tls13"
}
