path "apps/+/data/*" {
  capabilities = ["create", "read", "update", "delete"]
}

path "apps/+/metadata" {
  capabilities = ["list"]
}

path "apps/+/metadata/*" {
  capabilities = ["list", "read"]
}

path "sys/internal/ui/mounts" {
  capabilities = ["read", "list"]
}

path "sys/internal/ui/mounts/*" {
  capabilities = ["read", "list"]
}

path "sys/internal/ui/resultant-acl" {
  capabilities = ["read"]
}

path "sys/health" {
  capabilities = ["read"]
}
