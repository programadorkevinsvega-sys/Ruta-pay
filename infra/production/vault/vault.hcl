ui = true
disable_mlock = true

api_addr = "https://interserver-vault.tail4eafc5.ts.net:8200"
cluster_addr = "https://100.110.114.7:8201"
cluster_name = "shared-vault"

storage "raft" {
  path    = "/opt/vault/data"
  node_id = "shared-vault-1"
}

listener "tcp" {
  address         = "100.110.114.7:8200"
  cluster_address = "100.110.114.7:8201"
  tls_cert_file   = "/opt/vault/tls/tailscale.crt"
  tls_key_file    = "/opt/vault/tls/tailscale.key"
  tls_min_version = "tls12"
  tls_max_version = "tls13"
}
