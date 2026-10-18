# Retained API database bootstrap review

C1 finding accepted: boolean existence checks could miss extra sessions or bindings. C2 captures exact ordered scoped membership, binding and session identities, asserting one original session and comparing after bootstrap, reconnect, authenticated polling and completion.

Fresh C2 review: **None** (no required findings), under the explicit caller-owned test-server lifetime contract. The attach helper performs no migration, provisioning, grant or fallback creation. Root changed only the module wording from API process to API instance and formatted the reviewed source.

The executed C1 behavioral RED uses the original fresh-database constructor and fails with public HTTP 401 versus 200; it does not execute C2's stronger immediate witness. C2 RED patch is preserved but was not executed. No old binary or OS-process restart is claimed here.
