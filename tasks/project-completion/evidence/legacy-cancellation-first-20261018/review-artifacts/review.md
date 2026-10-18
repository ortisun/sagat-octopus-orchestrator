# D2 cancellation-first C1 reconciliation

Fresh decision review of D2 section5 returned None; public cancel is source-reachable without full data hydration. Code reviews split public operation receipt/audit (None), cancelled drain/report, and compensation/coordinator. The latter two found the same audit-oracle omission: only existing_outputs was excluded at finality and no total audit guard followed waiting/replays. Classified actionable, corrected once in C2.

C2 adds cancellation-specific total data-limit audit checks: zero after live wait, exactly one after genuine success, completion replay, operation replay and final compensation. Existing exact report-refusal provenance remains. Final other-audit query uses IS DISTINCT FROM the sole permitted refusal reason, catching stopped_for_run_data_limit and NULL reasons too. C1 artifacts unchanged; visible semantic delta is against root-formatted C1. No actual runtime RED is claimed for this static finding.

Root alone applies/executes. GPT-5.6 Sol temporary draft; fresh GPT-6 Astra reviews. External optional second review declined earlier. No production hook/grant/semantics change.

Fresh C2 audit-only delta review returned None: zero after waiting and exactly one after success/both replays/finalization, exact original refusal provenance retained, IS DISTINCT FROM catches every other or NULL reason. Stop condition met. C1 strict Clippy passed16.37s; no C1 runtime test was run. Root applies C2 before first actual old/current compatibility execution.
