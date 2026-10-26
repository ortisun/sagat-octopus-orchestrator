# Bounded response resource probe

Date: 2026-10-17. Native debug test binary; unchanged HTTP delivery implementation.


On the same Darwin/Apple M3 Max host, the native HTTP slow-reader unit fixture was run
as a standalone test executable under `/usr/bin/time -l`: one 4 MiB retained response,
one transfer permit, a 1 KiB TCP receive buffer, periodic reader progress and a 500 ms
absolute delivery budget. It passed in 0.91 s and recovered the permit before the
900 ms assertion while the client still held its socket. Maximum RSS of that **test
process** was **15,564,800 bytes** (14.84 MiB); reported peak memory footprint was
6,603,160 bytes. This includes the native server/client fixture and test runtime,
not a production control plane or database. It is not a multi-tenant capacity estimate.

Reproduce by building `sagat-api-http --lib` tests, locating the emitted test executable,
and running it directly under the OS resource meter with filter
`absolute_deadline_drops_buffered_bytes_and_recovers_slot_for_a_slow_reader`.
