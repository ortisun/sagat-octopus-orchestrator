# KV v2 mount secret. Change the prefix if the operator chooses another mount.
path "secret/data/sagat/credentials/*" { capabilities = ["create", "read"] }
path "auth/token/lookup-self" { capabilities = ["read"] }
path "auth/token/renew-self" { capabilities = ["update"] }
path "auth/token/revoke-self" { capabilities = ["update"] }
