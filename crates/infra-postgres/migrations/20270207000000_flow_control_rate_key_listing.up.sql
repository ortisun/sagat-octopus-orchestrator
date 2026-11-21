-- Feature 022, release 0.3.0 slice (T022-078; FR-022-048; plan "Visibility of the rate gate", decision D022-067): a page
-- of `GET /namespaces/{namespaceId}/rate-limit-keys` costs what its own keys cost, whatever the number of window rows of
-- the namespace, as decision D022-058 made the concurrency listing.
--
-- The listing orders windows by their oldest wait. The oldest wait of a window is kept on its row (`oldest_wait_at`), by
-- the trigger that already keeps its counters, in the transaction that adds or removes a waiter; the listing walks an
-- index in its own order and stops after the page.
--
-- Additive: one nullable column (no rewrite), two indexes on a table that is empty in every released deployment
-- (D022-037), and the count trigger's function replaced in place (same name, owner, `SECURITY DEFINER` and pinned path;
-- what it keeps of the grants is unchanged). The CHECK is added `NOT VALID` and validated by the next migration.

ALTER TABLE rate_limit_windows ADD COLUMN oldest_wait_at timestamptz;

UPDATE rate_limit_windows w
   SET oldest_wait_at = (SELECT min(x.enqueued_at) FROM rate_limit_waiters x
                          WHERE x.tenant_id = w.tenant_id AND x.namespace_id = w.namespace_id AND x.pool = w.pool
                            AND x.limit_scope = w.limit_scope AND x.group_name = w.group_name
                            AND x.key_digest = w.key_digest);

-- a window somebody waits for has an oldest wait, and one nobody waits for has none
ALTER TABLE rate_limit_windows ADD CONSTRAINT rate_limit_windows_oldest_wait_iff_waiters
    CHECK ((waiters = 0) = (oldest_wait_at IS NULL)) NOT VALID;

CREATE OR REPLACE FUNCTION rate_limit_window_count() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    row_of  record;
    delta   integer := CASE TG_OP WHEN 'INSERT' THEN 1 ELSE -1 END;
    grant_r boolean := TG_TABLE_NAME = 'rate_limit_grants';
    changed integer;
BEGIN
    IF TG_OP = 'INSERT' THEN row_of := NEW; ELSE row_of := OLD; END IF;
    IF grant_r THEN
        UPDATE public.rate_limit_windows w
           SET live_grants = w.live_grants + delta,
               grants_total = w.grants_total + CASE WHEN TG_OP = 'INSERT' THEN 1 ELSE 0 END,
               max_period_ms = CASE WHEN TG_OP = 'INSERT' THEN greatest(w.max_period_ms, row_of.period_ms)
                                    ELSE w.max_period_ms END,
               updated_at = clock_timestamp()
         WHERE w.tenant_id = row_of.tenant_id AND w.namespace_id = row_of.namespace_id AND w.pool = row_of.pool
           AND w.limit_scope = row_of.limit_scope AND w.group_name = row_of.group_name
           AND w.key_digest = row_of.key_digest;
    ELSE
        -- The window row is locked first, by its own statement: under READ COMMITTED the next statement then takes a
        -- snapshot in which every waiter another transaction committed for this key is visible (they wrote it under
        -- this lock), so the oldest wait recomputed below is never read from an older snapshot.
        PERFORM 1 FROM public.rate_limit_windows w
         WHERE w.tenant_id = row_of.tenant_id AND w.namespace_id = row_of.namespace_id AND w.pool = row_of.pool
           AND w.limit_scope = row_of.limit_scope AND w.group_name = row_of.group_name
           AND w.key_digest = row_of.key_digest
           FOR UPDATE;
        -- One statement may add and remove several waiters of a key: its row triggers fire after it, each seeing all of
        -- its rows, while the counter moves by one per trigger. A removal takes the oldest of the rows that remain unless
        -- the counter reaches 0 (then none), and keeps the value it had while the counter is above 0 and the remaining
        -- rows are gone; an addition lowers it to its own instant. After the last trigger the counter is the number of
        -- rows and the value their oldest, and every trigger leaves the CHECK true (as `concurrency_semaphore_count`).
        UPDATE public.rate_limit_windows w
           SET waiters = w.waiters + delta,
               oldest_wait_at = CASE
                   WHEN TG_OP = 'INSERT' THEN LEAST(w.oldest_wait_at, row_of.enqueued_at)
                   WHEN w.waiters + delta = 0 THEN NULL
                   ELSE COALESCE((SELECT min(x.enqueued_at) FROM public.rate_limit_waiters x
                                   WHERE x.tenant_id = row_of.tenant_id AND x.namespace_id = row_of.namespace_id
                                     AND x.pool = row_of.pool AND x.limit_scope = row_of.limit_scope
                                     AND x.group_name = row_of.group_name AND x.key_digest = row_of.key_digest),
                                 w.oldest_wait_at)
               END,
               max_period_ms = CASE WHEN TG_OP = 'INSERT' THEN greatest(w.max_period_ms, row_of.period_ms)
                                    ELSE w.max_period_ms END,
               updated_at = clock_timestamp()
         WHERE w.tenant_id = row_of.tenant_id AND w.namespace_id = row_of.namespace_id AND w.pool = row_of.pool
           AND w.limit_scope = row_of.limit_scope AND w.group_name = row_of.group_name
           AND w.key_digest = row_of.key_digest;
    END IF;
    GET DIAGNOSTICS changed = ROW_COUNT;
    IF changed <> 1 THEN
        RAISE EXCEPTION 'a rate-limit grant or waiter has no window row'
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION rate_limit_window_count() FROM PUBLIC;

-- The listing's order (`flow_control.rs`, `rate_limit_keys`): oldest wait first, a window nobody waits for last, then the
-- row's key within the namespace. The second index serves the listing of one group in the same order.
CREATE INDEX rate_limit_windows_listing_idx ON rate_limit_windows
    (tenant_id, namespace_id, (COALESCE(oldest_wait_at, 'infinity'::timestamptz)), pool, limit_scope, group_name,
     key_digest);
CREATE INDEX rate_limit_windows_group_listing_idx ON rate_limit_windows
    (tenant_id, namespace_id, group_name, (COALESCE(oldest_wait_at, 'infinity'::timestamptz)), pool, limit_scope,
     key_digest);
