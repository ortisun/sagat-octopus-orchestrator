# Disaster recovery drill (feature 010)

Run by `infra/dr/drill.sh` on 2026-10-11 with postgres (PostgreSQL) 18.6 (Homebrew), commit c33a514, on Darwin 25.3.0 arm64.
The clusters, the archive and the standbys ran on one machine; the numbers are of this procedure on this
machine, not an objective for your deployment (see docs/runbooks/disaster-recovery.md for how to run it
against yours). A real restore adds the time to fetch the backup from where it is kept.

### 1. Point-in-time recovery

| Step | Result |
|---|---|
| Population | 3000 runs before the base backup, 3000 during it, 1500 after the restore point (a third of the first two phases carried through their activity) |
| Base backup, taken while phase B ran | 0.4 s (`pg_basebackup -X stream`) |
| WAL segments archived at the disaster | 7 (16 MiB each; `archive_timeout = 30s`) |
| The mistake | `DROP SCHEMA public CASCADE` after the restore point, on the primary |
| Restored to | the named restore point `drill_target`: 6000 runs, 26000 journal events |
| Copy of the base backup | 0.4 s |
| WAL replay and promotion, until the database answers | 0.4 s |
| **Time to restore** (copy, replay, promote) | **0.8 s** |
| Journal digest before and after | **yes** (counts and sha256 of the journal and of the audit trail) |
| Replay of every run against its projections | `verified=6000 failed=0` |
| Discarded by the restore | the 1500 runs of phase C: what is written after the point is gone, by definition |

Digest before the disaster:

```
runs=6000
journal_events=26000
audit_records=6021
journal_sha256=1e6c402677612c64912b788862d5e7bb8e967a3c2538060537bde8ae57e53d61
audit_sha256=65ad5535a82689dbe292bb3884997c0473298cc576afb1f35048faa1911a0f6a
```

Digest of the restored database:

```
runs=6000
journal_events=26000
audit_records=6021
journal_sha256=1e6c402677612c64912b788862d5e7bb8e967a3c2538060537bde8ae57e53d61
audit_sha256=65ad5535a82689dbe292bb3884997c0473298cc576afb1f35048faa1911a0f6a
```

### 2. Failover to a standby (`kill -9` of the primary while the platform acknowledges runs)

| Standby | State reported by the primary | Time to promote (RTO of the database) | After the failover |
|---|---|---:|---|
| synchronous | sync | 0.2 s | verified=9816 failed=0 acknowledged=9814 present=9814 lost=0  |
| asynchronous | async | 0.1 s | verified=10288 failed=0 acknowledged=10286 present=10286 lost=0  |

What these rows say, and what they do not:

- The synchronous standby loses nothing, as it must: the primary does not acknowledge a commit before the standby has it, so what the platform acknowledged is there (RPO zero for acknowledged work). The drill fails if that ever stops being true.
- The asynchronous standby lost nothing *here*, on a loopback where it is never behind. That does not show that an asynchronous standby is safe: its RPO is the replication lag at the moment of the loss, which a loopback does not have. Measure the lag on your network (`pg_stat_replication`) and treat the loss of the last seconds of acknowledged runs as possible.
- The time to promote is the database's (the kill until the promoted standby answers). The platform's own recovery after it is the keeper's: another replica takes the dead replica's shards when its lease ends (15 s by default) and the passes are idempotent (docs/runbooks/scaling.md); the engine reconnects by itself.


