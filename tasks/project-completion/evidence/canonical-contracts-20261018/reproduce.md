# Canonical contract checkpoint

Baseb6e83866. HostNode26.11.1 andprotoc36.2 (CI usesNode22/Linux); no exact CI platform parity
claimed. CI command versions retained: Redocly@1 resolves1.34.21, AsyncAPI3.6.0, ajv-cli@5 with
ajv-formats@3. npm_config_ignore_scripts=true for these commands, explicitignore-scriptsforAsyncAPI.
Npx transitive dependencies are not locked; no CLI upgrade, SDK baseline blessing or format migration.

Initial OpenAPI lint exit0 with7warnings: missingexistingprojectMITlicense metadata and6tag
descriptions. Added those9metadata lines, using rootLICENSE/Cargo.toml's existingMITlicense.
Paths/components from paths: onward are byte-identical (openapi-metadata-check.json). Final
OpenAPI lint exit0 without warnings. AsyncAPI exit0,0errors/0warnings plus1latest-versioninfo;
no version conversion. protoc descriptor compilation exits0. Payment-saga example validates the
canonical workflow schema using the CI documented strict=false customkeyword policy.
Rust contract_workflow_primitives47/47 in1.00s, including directprotobuf compilation, schema/
identity ceilings, publicIR/goldens and closedAPIerror set, beforemetadata-onlyaddition. No SDK
client execution inferred. Exact calls/exits/wall times in cli-commands.json/rust-command.json;
initial/final sources and logs retained. No runtime API or semantic contract fields changed.
No fullworkspace/SDK58/Compose/resource/release acceptance; T014-043 stays open.
