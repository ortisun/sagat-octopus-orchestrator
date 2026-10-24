# Old nonfinal producer C1 review

Fresh Astra xhigh driver/bootstrap/role/private-handoff review returned None. Separate fresh oracle review found two actionable gaps: stored aggregate/checkpoint/work payload digests were never recomputed, and historical closure omitted exact CHECKPOINTED members plus checkpoint/finish/routing relationships. C2 recomputes every actual tenant payload SHA256 and checks twenty coordinates against source work/Return/checkpoint/journal identities, twenty total historical members and ten exact successful-finish/routing sequence pairs. Pending Return expiry/failure NULL checks are also explicit. Old production code is unchanged.

The initial C1 old-archive build passes47.29s; no C1 runtime/RED was executed. Any real failure is retained. The optional external review remains declined.

Source-grounded consumer contract correction: the archived design's claim that completed_count stays0 after member drain is incorrect. Current primitives.rs updates completed_count+1 while marking the admitted member DRAINED (around2950). Preserve0 at first DrainStarted, then require1settled/0active after ordinary drain, still0checkpoints and index1neveradmitted. This is a correction of test-plan vocabulary to existing native semantics, not a production/product change. The original plan snapshot remains immutable; a separate delivery note records the correction.
