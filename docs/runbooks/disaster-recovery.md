# Runbook: backup, point-in-time recovery, failover and the drill (feature 010)

For the people who run the platform's database. It says what has to survive a disaster in a single region, how to back it up, how to restore it to a point in time, how to fail over to a standby, what the platform does by itself when it comes back, and how to **drill** all of it. The multi-region case is out of scope (`spec.md`, Assumptions).

**Reference objectives (owner decision, 2026-10-16).** For the reference topology, the Helm chart with PostgreSQL and a **synchronous** standby: **RPO 0** for work the platform acknowledged, **failover RTO ≤ 5 minutes** (promotion plus shard takeover) and **point-in-time restore RTO ≤ 1 hour**. The drill below measures them and is meant to gate a release (the gate is not wired yet). Another topology states its own figures; with an **asynchronous** standby the RPO is the replication lag at the moment of the loss, which you measure on your network. What this page gives is the procedure, the drill that measures it on your deployment, and the measurements of the drill on one machine as an example of what is reported: [`tests/performance/reports/disaster-recovery.md`](../../tests/performance/reports/disaster-recovery.md).

Design: [ADR-0020](../adr/ADR-0020-the-scheduler-is-sharded-by-run-and-owned-by-lease-with-a-fence.md); the engine at more than one replica: [`scaling.md`](scaling.md).

## 1. What there is to lose

| What | Where | Lost means | Kept by |
|---|---|---|---|
| Runs, steps, the journal, the audit trail, definitions, quotas, bindings, alerts | PostgreSQL | the platform | base backups and the WAL (section 3), a standby (section 5) |
| Secrets (sealed values) | PostgreSQL | the values | the same; **and the key ring** (`SAGAT_SECRETS_KEYS`), which is *not* in the database: a restored database without its key ring holds secrets nobody can open. Keep the ring where the backups are not, and test opening a secret after every restore drill |
| Script artifacts and the large payloads | the object store | the artifacts, and the runs that pin them | the object store's own versioning and replication (not covered here, G251) |
| The signing key of the artifacts (`SAGAT_SCRIPT_SIGNING_KEY`) | the operator's secret store | the ability to publish new script versions; old artifacts still verify with the public key | the operator |
| Hints and run events in NATS | the broker | nothing: the outbox in PostgreSQL is the authority and republishes | nothing needed |
| Workflow definitions as files | the operator's repository (feature 011) | nothing | git |

The platform keeps no state of its own in an engine or a control-plane process. Losing every process loses nothing; losing the database loses everything that is not in a backup.

## 2. What a restore means for a run

A point-in-time restore takes the database back to a moment. The platform has the same runs it had then, in the same states, and **no memory of what happened after**. Three consequences, none of them a defect:

- **Work done after the point is redone.** A step that a worker completed after the point is `READY` or `LEASED` again; its worker's answer, if it arrives, is refused as stale (the lease it holds is not the database's). External effects the step caused are *not* undone and not known: a payment charged after the point is charged again unless the activity is idempotent with the provider's key, or a reconciliation asks the provider first. This is the platform's general rule for external effects (at least once, `AGENTS.md` rule 4) made visible by the restore; **before you restore, tell the owners of the activities that run on the platform what time you are going back to.**
- **Timers fire late, once.** A timer that fell due between the point and now fires at once, when the scheduler first looks.
- **Hints are republished.** The outbox is restored with its undelivered messages; those delivered after the point are published again with the same ids and the broker drops the duplicates it still remembers.

The journal and the audit trail are append-only, so a restored database has *exactly* the rows it had at the point: that is what the drill checks.

## 3. Backups

Two things are needed, continuously: a **base backup** at some interval and the **WAL archive** since. Restoring to any moment after the base backup is replaying the archive up to it.

- Base backups: `pg_basebackup -D <dir> -Fp -X stream -c fast` (or your platform's snapshot tool), at an interval that keeps replay short (daily is a start; the replay is the WAL since, at the rate the platform writes it: measure it, section 6).
- WAL archive: `archive_mode = on`, an `archive_command` that copies a segment to storage **outside the failure domain of the database** and does not overwrite, `archive_timeout` to bound how long a quiet database keeps its last transactions out of the archive (the bound of the RPO of an archive-only recovery). Managed services and tools such as pgBackRest, WAL-G or Barman do this and verify it; use one.
- Retention: at least the oldest point you will want to restore to. A run that is waiting is a row in the live tables, not in the archive, so retention is about *mistakes you may discover late*, not about waiting runs.
- Verify what you took: `pg_verifybackup` on each base backup, and **restore it** (section 6). An untested backup is a hope.
- Encrypt at rest and in transit; the backups hold payload digests, audit records, sealed secrets and the content of payloads that were stored inline.

## 4. Point-in-time recovery

```sh
# 0. Decide the point: a time (recovery_target_time), or a name you created before a risky change:
#      SELECT pg_create_restore_point('before-migration-42');
# 1. Stop the platform's writers (engines, control planes) so that nothing writes to the damaged database.
# 2. Restore the newest base backup taken BEFORE the point into an empty directory.
cp -R /backups/base-2026-10-12 /var/lib/postgresql/restore && chmod 700 /var/lib/postgresql/restore
# 3. Tell the server how to fetch WAL and where to stop.
cat >> /var/lib/postgresql/restore/postgresql.conf <<'CONF'
restore_command = 'cp /archive/%f "%p"'          # or the tool's: wal-g wal-fetch %f %p
recovery_target_name = 'before-migration-42'      # or recovery_target_time = '2026-10-12 14:03:00+00'
recovery_target_action = 'promote'
CONF
touch /var/lib/postgresql/restore/recovery.signal
# 4. Start it. It replays the archive up to the point, then promotes itself.
pg_ctl -D /var/lib/postgresql/restore start
# 5. Check, before anything points at it (section 7), then point the platform at it.
```

Recovery is slower the older the base backup and the busier the platform was; the drill measures it. Do not point the platform at a server that is still in recovery.

## 5. Failover to a standby

A streaming standby (`pg_basebackup -R`) is the answer to a lost primary when you cannot wait for a restore. The choice that sets the loss:

- **Synchronous** (`synchronous_standby_names`): a transaction is acknowledged after the standby has it. Losing the primary loses no acknowledged transaction (the drill shows it: every run the platform said yes to is on the promoted standby); a standby that is down stops commits unless you accept a degraded mode, so run two standbys with `FIRST 1 (a, b)` or `ANY 1 (a, b)`.
- **Asynchronous**: faster commits, and the transactions of the last moments before the primary died may not have reached the standby. The drill reports how many acknowledged runs an asynchronous standby lost in its run; it can be none and it can be some, which is why it is a risk to take on purpose.

To fail over: make sure the old primary cannot come back as a primary (power it off, fence it: if it returns it must be rebuilt as a standby with `pg_rewind` or a new base backup), `pg_ctl promote` the standby, point the platform at it (the address your engines and control planes use must resolve to the new primary: a service name, a virtual IP, a pooler, a DNS record), and check section 7. The engines reconnect by themselves (the pool replaces broken connections; the chaos suite kills every connection under load and loses nothing), and the shard leases are in the database: the leases of the dead primary's era are expired or, if the standby was behind, are ones the standby knows; a replica that finds its lease is not the database's takes the shard again with a new generation, and the **owner in the fence** is the replica's name with a random suffix, so a generation that goes round again can never be mistaken for the one it replaced.

## 6. The drill

Quarterly, and after any change to backups, replication or the platform's major version, in an environment that is not production, with a population like production's.

`infra/dr/drill.sh` makes both disasters on real PostgreSQL clusters it starts itself and reports what happened (`SAGAT_DR_RUNS` sizes the population):

1. **Point-in-time recovery.** A workload runs while a base backup is taken; a restore point is named; more work is done; the schema is destroyed (an operator's mistake); a cluster is restored from the base backup and the archive to the restore point. It checks that the **journal digest** (`infra/dr/journal_digest.sql`: counts and a sha256 of the journal and of the audit trail) is identical before and after, and that the journal of **every run replays to its projections** (`PgRunStore::verify_run`). It reports the time to restore and what the restore discarded.
2. **Failover.** A primary with a standby is killed with `kill -9` while the platform acknowledges runs as fast as it can, the standby is promoted, and every acknowledged run is looked for. With a synchronous standby none may be missing (the script fails otherwise); with an asynchronous one the loss is reported. It reports the time to promote.

To drill your own deployment, do the same by hand with your tools and keep the evidence: the digest before and after (`psql -X -At -f infra/dr/journal_digest.sql <database>`), the count and result of `verify_run` for every run (the `dr_verify` test does it for a database URL: `SAGAT_DR_URL=... cargo test -p sagat-tests --test perf_scalability_ha --release -- --ignored dr_verify`), the time from the decision to restore to the first successful `GET /v1/health` through the platform, and one secret opened with the restored key ring.

**What the drill does not cover** (G250, G251): the object store and its replication; the broker; the key ring's escrow; a standby in another zone; the time to fetch a base backup from remote storage; engines reconnecting through *your* address indirection. Add those to yours.

## 7. After a restore or a failover

1. `SELECT count(*) FROM scheduler_shards WHERE expires_at > now()`: after the engines are up it is 64 (they take the shards by themselves). `scheduler_shards_owned` sums to 64.
2. `SELECT status, count(*) FROM runs WHERE finished_at IS NULL GROUP BY 1`: the live runs are what you expect for the point.
3. The digest and the replay of section 6, on a sample or on everything.
4. Open one secret through the API (the key ring is the right one).
5. Start the engines, then the control planes. Watch `scheduler_lag_seconds` fall (timers that fell due meanwhile fire at once), `leases_expired` go to zero (attempts whose workers are gone are taken back and retried), and `outbox_backlog` drain.
6. Tell the owners of the activities the time you went back to (section 2).
