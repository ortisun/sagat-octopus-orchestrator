//! A kill switch inside PostgreSQL: the connection of a transaction dies right after the Nth
//! statement that writes, as when the process is killed between two statements of it.
//!
//! The switch is a statement-level trigger on every table of the schema. It counts the writing
//! statements of the transaction (a transaction-local setting, so nothing leaks between
//! transactions), and on the armed one it terminates its own backend. PostgreSQL then rolls the
//! transaction back exactly as it would for a killed process; the pool sees a dead connection.
//! Statements that only read or lock are not counted: a death before the first write is the same
//! observable state as no attempt.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use sqlx::PgPool;

pub struct Tripwire {
    pool: PgPool,
}

impl Tripwire {
    /// Installs the switch, disarmed.
    pub async fn install(pool: &PgPool) -> Self {
        for statement in [
            "CREATE TABLE fault_plan (kill_at int, watching boolean NOT NULL DEFAULT false, kill_table text)",
            "INSERT INTO fault_plan (kill_at, watching) VALUES (NULL, false)",
            "CREATE TABLE fault_log (id bigserial PRIMARY KEY, xid bigint NOT NULL, \
             statement int NOT NULL, table_name text NOT NULL)",
            "CREATE FUNCTION fault_tripwire() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER \
             SET search_path = public, pg_catalog AS $f$ \
             DECLARE n int; target int; watch boolean; ktable text; \
             BEGIN \
               n := coalesce(nullif(current_setting('fault.statements', true), ''), '0')::int + 1; \
               PERFORM set_config('fault.statements', n::text, true); \
               SELECT kill_at, watching, kill_table INTO target, watch, ktable FROM fault_plan; \
               IF watch THEN \
                 INSERT INTO fault_log (xid, statement, table_name) \
                 VALUES (txid_current(), n, TG_TABLE_NAME); \
               END IF; \
               IF target IS NOT NULL AND n = target AND (ktable IS NULL OR ktable = TG_TABLE_NAME) THEN \
                 PERFORM pg_terminate_backend(pg_backend_pid()); \
                 PERFORM pg_sleep(30); \
               END IF; \
               RETURN NULL; \
             END $f$",
            "DO $d$ DECLARE t text; BEGIN \
               FOR t IN SELECT tablename FROM pg_tables \
                        WHERE schemaname = 'public' AND tablename NOT LIKE 'fault\\_%' \
                          AND tablename <> '_sqlx_migrations' \
               LOOP \
                 EXECUTE format('CREATE TRIGGER fault_tripwire AFTER INSERT OR UPDATE OR DELETE \
                                 ON %I FOR EACH STATEMENT EXECUTE FUNCTION fault_tripwire()', t); \
               END LOOP; \
             END $d$",
        ] {
            sqlx::query(statement)
                .execute(pool)
                .await
                .unwrap_or_else(|e| panic!("{statement}: {e}"));
        }
        Self { pool: pool.clone() }
    }

    /// Records the writing statements of the transactions that follow, without killing anything.
    pub async fn watch(&self) {
        self.set("UPDATE fault_plan SET kill_at = NULL, watching = true, kill_table = NULL")
            .await;
    }

    /// Kills the connection of the next transaction that makes `statement` writing statements.
    pub async fn arm(&self, statement: usize) {
        sqlx::query("UPDATE fault_plan SET kill_at = $1, watching = false, kill_table = NULL")
            .bind(i32::try_from(statement).expect("statement"))
            .execute(&self.pool)
            .await
            .expect("arm");
    }

    /// Kills the connection of the next transaction whose `statement`th writing statement writes `table`: the death of a process
    /// that already committed what came before (the run, when the table is the one a later, separate statement writes).
    pub async fn arm_on(&self, table: &str, statement: usize) {
        sqlx::query("UPDATE fault_plan SET kill_at = $1, watching = false, kill_table = $2")
            .bind(i32::try_from(statement).expect("statement"))
            .bind(table)
            .execute(&self.pool)
            .await
            .expect("arm");
    }

    pub async fn disarm(&self) {
        self.set("UPDATE fault_plan SET kill_at = NULL, watching = false, kill_table = NULL")
            .await;
    }

    async fn set(&self, statement: &'static str) {
        sqlx::query(statement)
            .execute(&self.pool)
            .await
            .expect("plan");
    }

    /// The writing statements the watched transactions made: transaction, position, table.
    pub async fn writes(&self) -> Vec<(i64, usize, String)> {
        let rows: Vec<(i64, i32, String)> =
            sqlx::query_as("SELECT xid, statement, table_name FROM fault_log ORDER BY id")
                .fetch_all(&self.pool)
                .await
                .expect("log");
        rows.into_iter()
            .map(|(xid, n, table)| (xid, usize::try_from(n).expect("position"), table))
            .collect()
    }
}

/// A digest of the content of every table of the schema except `schedules` (whose row the retry
/// bookkeeping of a failed step legitimately updates; the test compares its columns itself). Two
/// snapshots that are equal prove that no row of any other table was inserted, deleted or changed;
/// the switch's own tables are left out.
pub async fn table_digests(pool: &PgPool) -> BTreeMap<String, String> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT tablename::text, \
                (xpath('/row/c/text()', query_to_xml( \
                    format('SELECT md5(coalesce(string_agg(t::text, '''' ORDER BY t::text), '''')) AS c FROM %I t', \
                           tablename), false, true, '')))[1]::text \
         FROM pg_tables \
         WHERE schemaname = 'public' AND tablename NOT LIKE 'fault\\_%' AND tablename <> 'schedules'",
    )
    .fetch_all(pool)
    .await
    .expect("table digests")
    .into_iter()
    .collect()
}
