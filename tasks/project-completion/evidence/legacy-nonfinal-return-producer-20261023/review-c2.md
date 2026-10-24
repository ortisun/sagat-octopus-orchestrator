# Exact coordinate correction

Fresh C2 oracle review found one actionable gap: member/checkpoint IDs and equal payload digests could point to another successful iteration's Work/Return. C3 requires exact legacyWork/legacyReturned node names, owner-root paths, iteration index paths and member entry/return metadata equality. It retains the full C2 checkpoint/journal/digest/finish/routing relationships.

Preliminary C2 actual old owner producer passes1/1,9.83s wall/8.35s harness, on immutable production6c44 with owner API/store postgres. It observes L2100001,V2100030,E4200031,20checkpoints, pending nonfinal Return and no live work. This is not final acceptance or a RED. C3 must be compiled/executed with its exact producer bytes before standing.
