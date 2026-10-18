# Refused-output compensation worker C2

- Base HEAD: `1310b09b06e0fcf04e26c4b23a3ea8c4cbb2e6db`.
- Base byte-ceiling SHA-256: `7b28f7f21de920acb75474e7359eee45b49ad8d125f9e97968f7c7109ca18e26`.
- C1 SHA-256: `3473238b93be5c8d6c6055bd00b2fea38e0f549d6e80182b63f660092c7191d1`.
- C2 patch: `/tmp/sagat-refused-output-compensation-worker-tests-c2.patch`.
- C2 SHA-256: `e21e36a069821b0da4a58f83c08fc77ce2f99066362fe902d2eadc53318ba642`.
- Temporary materialization was created under `/tmp` for apply checks and removed afterward.

The sole change from C1 removes the nonempty target-key assertion and adds exact equality of the refused compensation target key and full target instance with its actual forward lease. The two assertions run immediately after extracting the recorded missing-input lease. No other behavior changed.

Artifact checks passed: C1 applies to current HEAD; C2 `git apply --check --whitespace=error` passes against current HEAD; applied C2 is byte-identical to the draft; repository byte-ceiling file remains unchanged. `git diff --no-index --check` produced no diagnostics (exit 1 for the differing files); the explicit apply whitespace check passed. No repository edits, tests, builds, formatter, commits or subagents.

```diff
--- C1
+++ C2
@@ -715,7 +715,6 @@
                         "the refused forward output must not be defaulted or synthesized"
                     );
                     assert_eq!(task.target_node, "reserve");
-                    assert!(!task.target_idempotency_key.is_empty());
                     assert!(task.idempotency_key.starts_with("compensation:"));
                     let instance = task.instance.as_ref().expect("compensation instance");
                     let target = task.target_instance.as_ref().expect("target instance");
@@ -773,6 +772,8 @@
         )
         .await;
         let missing = missing.expect("the refused member's compensation reported its input error");
+        assert_eq!(missing.target_idempotency_key, forward.idempotency_key);
+        assert_eq!(missing.target_instance, forward.instance);
         flow.drive().await;
 
         let open: (
```
