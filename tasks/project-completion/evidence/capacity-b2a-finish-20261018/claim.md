CLAIM: A dormant native exact-current-child Finish can persist bounded A1 metadata and one
durable receipt/journal/audit/outbox result under existing own authority, without touching run
business state, production capacity accounting or runtime activation.
WHY IT MATTERS: Storage/receipt recovery must preserve logical chain finality, no double finish
and unknown COMMIT outcomes before any future producer or execution gate can depend on it.
