# Bounded test oracle reviews, cycles2 and3

Three independent cycle2 read-only reviews partitioned variants/bounds, qualified Run/receipt
binding, and retained guards/global keys/effective privileges. Root runs all actual tests.

- Guard/key/privilege review (Astra/xhigh): no actionable finding after cycle1 fixes.
- Binding review (Astra/xhigh): actionable missing tenant and group qualifiers. Removing either
  qualifier from both new referenced UNIQUE and scoped FK would survive old tests. Added two
  existing destination groups: same tenant/different group UUID; foreign tenant/same group UUID.
  All remaining receipt identity fields stay unchanged; assert23503 on the exact scoped FK.
- Variant review (Sol/high): actionable empty/leading-punctuation/ASCII/minimum-length gaps.
  Added empty, .owner and non-ASCII rejection for both member/receipt, plus successful one-character
  owner/index0 member and receipt, retaining64-character/index9999 positive endpoints.

Fresh cycle3 Astra/xhigh review of only these two amended oracle families found one actionable
fixture error: an empty destination group cannot be VACANT. Existing constraint requires RETIRED.
Root applies that direct fixture correction after preserving the frozen regression result.
No fourth review of these decisions, no fourth whole-protocol review. Actual tests remain the
required acceptance evidence. SQL storage contract independently had one review with no finding.
External review was declined by the user; no external CLI invoked.
