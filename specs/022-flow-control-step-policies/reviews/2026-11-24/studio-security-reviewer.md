# Security review of the Studio's flow-control views (2026-11-24)

Reviewer: the repository's read-only `security-reviewer`, given commits `460038af`, `b23b2219`, `53cf9750` and
`6811094c` (tasks T022-054, T022-055, T022-056, T022-085) and FR-022-022, FR-022-048, FR-022-057, NFR-022-002 and
decisions D022-055, D022-058 to D022-063. It read the code and ran nothing. No Blocker, no Major, one minor, four nits.

| Finding | Answer (delegate, OD-2026-11-24-4) | Evidence |
|---|---|---|
| minor 1: a member named `__proto__` in a policy replaced the policy's prototype in `ordered()` (`flowControl.ts`), so the editor showed values the saved draft did not hold | The policy is built from its entries (`Object.fromEntries`), and known members are read as own members only | `flowControl.test.ts` "keeps a member called __proto__ as an ordinary member and never as the policy prototype", red before |
| nit 1: `readPolicy` checked a concurrency policy against the rate-limit members | `readPolicy` takes the gate and names the members that gate does not know | "names a rate-limit member written on a concurrency policy as unknown", red before |
| nit 2: no estimate for a caller who may not see the label read "could not compute" | The rate view says the estimate is withheld when the label is hidden | `RunGates.test.tsx` "says an estimate withheld from a caller who may not see the label", red before |
| nit 3: a listing row without `keyLabel` showed an empty code box | Shown as "Unsupported (no label sent)", as the run view does (FR-007-010) | `FlowControlKeysPanel.test.tsx` "names a hidden label, scopes and pools in words", red before |
| nit 4: every 404 of a listing reads "this platform does not list" | Kept: a 404 for a namespace that disappeared also has nothing to list, and nothing is disclosed | none |

The reviewer found nothing in the other classes it checked: no key value is received or stored, a hidden label cannot
look like a key, the cursor is never parsed or built, no unsafe HTML or URL building, the namespace comes from the
server's own list, error messages are text, the journey holds no secret.
