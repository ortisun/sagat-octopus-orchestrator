Astra/xhigh fresh read-only implementation review: no code/hash/API mismatch. Two actionable
oracles: invalid commands never called fingerprint(), and owner mutation changed both bytes and
length. Root added fingerprint refusal assertions for derived owner/invalid/exhausted counters and
a same-length tasks/items variant, retaining item/items length mutation. No tests by reviewer.
