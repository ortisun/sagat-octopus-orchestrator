-- Feature 022, release 0.3.0 slice (T022-026, T022-105; FR-022-022; keys review 2026-11-20, architecture M3): a page of
-- `GET /namespaces/{namespaceId}/concurrency-keys` costs what its own keys cost, whatever the number of live keys of the
-- namespace (D022-058).
--
-- The listing orders keys by their oldest wait. It was computed per semaphore row by a correlated `min()` over the
-- waiters and sorted before `LIMIT`, so every page read every live key of the namespace. Now the oldest wait of a key is
-- kept on its semaphore row (`oldest_wait_at`), by the trigger that already keeps its counters, in the transaction that
-- adds or removes a waiter; the listing walks an index in its own order and stops after the page. The oldest holder of a
-- key is read through an index that ends with `acquired_at` (one probe per listed key).
--
-- Additive: one nullable column (no rewrite), two indexes on a table that is empty in every released deployment (D022-037),
-- one index on the holders replacing a prefix of it, and the count trigger's function replaced in place (same name,
-- owner, `SECURITY DEFINER` and pinned path). The CHECK is added `NOT VALID` and validated by the next migration.

ALTER TABLE concurrency_semaphores ADD COLUMN oldest_wait_at timestamptz;

UPDATE concurrency_semaphores s
   SET oldest_wait_at = (SELECT min(w.enqueued_at) FROM concurrency_waiters w
                          WHERE w.tenant_id = s.tenant_id AND w.namespace_id = s.namespace_id AND w.pool = s.pool
                            AND w.permit_scope = s.permit_scope AND w.group_name = s.group_name
                            AND w.key_digest = s.key_digest);

-- a key somebody waits for has an oldest wait, and a key nobody waits for has none
ALTER TABLE concurrency_semaphores ADD CONSTRAINT concurrency_semaphores_oldest_wait_iff_waiters
    CHECK ((waiters = 0) = (oldest_wait_at IS NULL)) NOT VALID;

CREATE OR REPLACE FUNCTION concurrency_semaphore_count() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    holder_delta integer := 0;
    waiter_delta integer := 0;
    k_tenant uuid; k_namespace uuid; k_pool text; k_scope text; k_group text; k_digest bytea;
    changed integer;
BEGIN
    IF TG_TABLE_NAME = 'concurrency_holders' THEN
        holder_delta := CASE TG_OP WHEN 'INSERT' THEN 1 ELSE -1 END;
    ELSE
        waiter_delta := CASE TG_OP WHEN 'INSERT' THEN 1 ELSE -1 END;
    END IF;
    -- a field is read only from the table that has it: PL/pgSQL resolves every field an expression names
    IF TG_OP = 'INSERT' THEN
        k_tenant := NEW.tenant_id; k_namespace := NEW.namespace_id; k_pool := NEW.pool; k_group := NEW.group_name;
        k_digest := NEW.key_digest;
        IF TG_TABLE_NAME = 'concurrency_holders' THEN
            k_scope := NEW.holder_scope;
        ELSE
            k_scope := NEW.permit_scope;
        END IF;
    ELSE
        k_tenant := OLD.tenant_id; k_namespace := OLD.namespace_id; k_pool := OLD.pool; k_group := OLD.group_name;
        k_digest := OLD.key_digest;
        IF TG_TABLE_NAME = 'concurrency_holders' THEN
            k_scope := OLD.holder_scope;
        ELSE
            k_scope := OLD.permit_scope;
        END IF;
    END IF;
    IF TG_TABLE_NAME = 'concurrency_holders' THEN
        UPDATE public.concurrency_semaphores s
           SET holders = s.holders + holder_delta,
               updated_at = clock_timestamp()
         WHERE s.tenant_id = k_tenant AND s.namespace_id = k_namespace AND s.pool = k_pool AND s.permit_scope = k_scope
           AND s.group_name = k_group AND s.key_digest = k_digest;
    ELSE
        -- The key's row is locked first, by its own statement: under READ COMMITTED the next statement then takes a
        -- snapshot in which every waiter another transaction committed for this key is visible (they wrote it under
        -- this lock), so the oldest wait recomputed below is never read from an older snapshot.
        PERFORM 1 FROM public.concurrency_semaphores s
         WHERE s.tenant_id = k_tenant AND s.namespace_id = k_namespace AND s.pool = k_pool AND s.permit_scope = k_scope
           AND s.group_name = k_group AND s.key_digest = k_digest
           FOR UPDATE;
        -- One statement may add and remove several waiters of a key (a run's waiters removed together, a waiter
        -- replaced): its row triggers fire after it, each seeing all of its rows, while the counter moves by one per
        -- trigger. So a removal takes the oldest of the rows that remain unless the counter reaches 0 (then none), and
        -- keeps the value it had while the counter is above 0 and the remaining rows are gone (a later removal of the
        -- same statement brings it to 0); an addition lowers it to its own instant. After the last trigger the counter
        -- is the number of rows and the value their oldest, and every trigger leaves the CHECK true.
        UPDATE public.concurrency_semaphores s
           SET waiters = s.waiters + waiter_delta,
               oldest_wait_at = CASE
                   WHEN TG_OP = 'INSERT' THEN LEAST(s.oldest_wait_at, NEW.enqueued_at)
                   WHEN s.waiters + waiter_delta = 0 THEN NULL
                   ELSE COALESCE((SELECT min(w.enqueued_at) FROM public.concurrency_waiters w
                                   WHERE w.tenant_id = k_tenant AND w.namespace_id = k_namespace AND w.pool = k_pool
                                     AND w.permit_scope = k_scope AND w.group_name = k_group
                                     AND w.key_digest = k_digest),
                                 s.oldest_wait_at)
               END,
               updated_at = clock_timestamp()
         WHERE s.tenant_id = k_tenant AND s.namespace_id = k_namespace AND s.pool = k_pool AND s.permit_scope = k_scope
           AND s.group_name = k_group AND s.key_digest = k_digest;
    END IF;
    GET DIAGNOSTICS changed = ROW_COUNT;
    IF changed <> 1 THEN
        RAISE EXCEPTION 'a concurrency holder or waiter has no semaphore row'
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION concurrency_semaphore_count() FROM PUBLIC;

-- The listing's order (`flow_control.rs`, `concurrency_keys`): oldest wait first, a key nobody waits for last, then the
-- row's key within the namespace. The second index serves the listing of one group in the same order.
CREATE INDEX concurrency_semaphores_listing_idx ON concurrency_semaphores
    (tenant_id, namespace_id, (COALESCE(oldest_wait_at, 'infinity'::timestamptz)), pool, permit_scope, group_name,
     key_digest);
CREATE INDEX concurrency_semaphores_group_listing_idx ON concurrency_semaphores
    (tenant_id, namespace_id, group_name, (COALESCE(oldest_wait_at, 'infinity'::timestamptz)), pool, permit_scope,
     key_digest);

-- The oldest holder of a key in one probe; the key prefix serves every lookup the replaced index served.
CREATE INDEX concurrency_holders_key_age_idx
    ON concurrency_holders (tenant_id, namespace_id, pool, holder_scope, group_name, key_digest, acquired_at);
DROP INDEX concurrency_holders_key_idx;
