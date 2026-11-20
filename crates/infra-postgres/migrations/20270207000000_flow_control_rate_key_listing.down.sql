-- Restores the schema before `20270207000000_flow_control_rate_key_listing`. Nothing is lost: `oldest_wait_at` is
-- derived from the waiters, which stay.
DROP INDEX rate_limit_windows_group_listing_idx;
DROP INDEX rate_limit_windows_listing_idx;

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
    UPDATE public.rate_limit_windows w
       SET waiters = w.waiters + CASE WHEN grant_r THEN 0 ELSE delta END,
           live_grants = w.live_grants + CASE WHEN grant_r THEN delta ELSE 0 END,
           grants_total = w.grants_total + CASE WHEN grant_r AND TG_OP = 'INSERT' THEN 1 ELSE 0 END,
           max_period_ms = CASE WHEN TG_OP = 'INSERT' THEN greatest(w.max_period_ms, row_of.period_ms)
                                ELSE w.max_period_ms END,
           updated_at = clock_timestamp()
     WHERE w.tenant_id = row_of.tenant_id AND w.namespace_id = row_of.namespace_id AND w.pool = row_of.pool
       AND w.limit_scope = row_of.limit_scope AND w.group_name = row_of.group_name AND w.key_digest = row_of.key_digest;
    GET DIAGNOSTICS changed = ROW_COUNT;
    IF changed <> 1 THEN
        RAISE EXCEPTION 'a rate-limit grant or waiter has no window row'
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION rate_limit_window_count() FROM PUBLIC;

ALTER TABLE rate_limit_windows DROP CONSTRAINT rate_limit_windows_oldest_wait_iff_waiters;
ALTER TABLE rate_limit_windows DROP COLUMN oldest_wait_at;
