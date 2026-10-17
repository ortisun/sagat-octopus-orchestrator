# No credential read or metadata deletion; retirement retains permanent CAS tombstones.
path "secret/metadata/sagat/credentials/*" { capabilities = ["read"] }
path "secret/data/sagat/credentials/*" { capabilities = ["create"] }
path "secret/destroy/sagat/credentials/*" { capabilities = ["update"] }
path "auth/token/lookup-self" { capabilities = ["read"] }
path "auth/token/renew-self" { capabilities = ["update"] }
path "auth/token/revoke-self" { capabilities = ["update"] }
