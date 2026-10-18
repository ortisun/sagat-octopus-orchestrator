
## Effective output scalar port declaration — 2026-10-18

RunTx now declares logical_output_bytes before staging, with checked complete-RunData compatibility default. Fresh one-file C1 None; all application targets compile under strict Clippy19.63 s and fmt passes,1109 inputs unchanged. No test execution or effective PostgreSQL behavior is claimed for this declaration. The mandatory PG override/overlay and consumer activation remain Q5/Q6b; no caller changed. Evidence: evidence/effective-output-scalar-port-20261018/manifest.json.
