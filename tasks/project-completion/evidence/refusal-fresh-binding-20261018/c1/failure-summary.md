# Actual C1 failure

One test ran and failed, exit101,58.94s including48.80s compilation; harness7.90s. Exact full-restoration barrier and native unbind completed. The stable post-unbind snapshot failed because physical tenant content usage changed. A value-free recursive comparison found only tenantRetainedUsage.content_bytes differs.

Source20261024000000_durable_quotas.up.sql532 accounts worker binding pattern+subject UTF-8 bytes and refunds these on deletion. The intended native unbind therefore changes this physical counter as well as the exact lease timestamps and audit. C1's plan omitted this legitimate effect. C2 must verify the exact pre-read matching binding byte refund and keep every other field unchanged. No production security failure or passing authority proof is inferred from this failed fixture.

C1's large assertion diagnostic included test bodies and credential hashes. Public evidence replaces those diagnostic lines; diagnostic-redaction.json records original SHA/size and value-free differing paths. Original raw output is task-private mode0600 outside the repository. No false raw-evidence claim or hash-bearing diagnostic is committed.
