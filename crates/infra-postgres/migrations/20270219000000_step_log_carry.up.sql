-- Feature 041 (step logs), task T041-36: the carry on a report (design section 2a, specs/041-step-logs/data-model.md
-- §8a, ADR-0095, ADR-0097). One call of `sagat_step_log_carry`, made by the report's transaction right before its
-- commit, writes the attempt's last records and its final mark; it never waits for a step-log lock and answers the
-- quota's refusals and a busy usage row itself, so that a carry never makes a report fail, block or retry.
--
-- Additive: functions only. Every function is `SECURITY INVOKER` (it runs with the runtime role's grants on the
-- step-log tables), has a pinned `search_path`, is revoked from `PUBLIC` and executable by `sagat_app` only.
--
-- The functions:
-- * `sagat_step_log_policy_fence_keys` and `sagat_step_log_stream_lock_key`: the keys of the six policy fences and of
--   the attempt's stream lock, built in SQL from the attempt's identity, never passed by a caller (security review Nit
--   S-4). A test requires them equal to the keys the ingest builds in Rust (`step_log_policy_fence_keys`,
--   `stream_lock_key`) for the same inputs (decision D041-146 (3)).
-- * `sagat_step_log_policies`: the three namespace resources resolved as `effective_policy` resolves them.
-- * `sagat_step_log_carry_plan`: T5 and T6 of a carry, exactly `sagat_domain::step_logs::plan_batch` (a property test
--   runs both on the same carries, decision D041-100), with no statement for a full carry.
-- * `sagat_step_log_carry_write`: CC to CE, called as it is on the fast path and inside the carry's block otherwise.
-- * `sagat_step_log_carry`: item 0, CA, CB, the reservation, the write and the block.
--
-- The function never names the usage row: a carry's charge takes a quota lease, the usage row only to refill, and it
-- reads the allowance through `sagat_quota_step_log_allowance` (FR-041-008; a source-text test holds it).

CREATE FUNCTION sagat_step_log_policy_fence_keys(p_tenant uuid, p_namespace uuid) RETURNS text[]
LANGUAGE sql IMMUTABLE SECURITY INVOKER SET search_path = pg_catalog, pg_temp AS $$
    SELECT ARRAY['quota-policy:step_log_attempt_records:' || p_tenant::text,
                 'quota-policy:step_log_attempt_records:' || p_tenant::text || ':' || p_namespace::text,
                 'quota-policy:step_log_attempt_bytes:' || p_tenant::text,
                 'quota-policy:step_log_attempt_bytes:' || p_tenant::text || ':' || p_namespace::text,
                 'quota-policy:step_log_retention_days:' || p_tenant::text,
                 'quota-policy:step_log_retention_days:' || p_tenant::text || ':' || p_namespace::text]
$$;

CREATE FUNCTION sagat_step_log_stream_lock_key(p_tenant uuid, p_run uuid, p_step uuid, p_attempt integer,
                                               p_generation bigint) RETURNS text
LANGUAGE sql IMMUTABLE SECURITY INVOKER SET search_path = pg_catalog, pg_temp AS $$
    SELECT 'step-log-stream:' || p_tenant::text || ':' || p_run::text || ':' || p_step::text || ':'
           || p_attempt::text || ':' || p_generation::text
$$;

-- The value in force of each resource: the namespace's setting, else the tenant's, else the ceiling; the ceiling is
-- what an operator granted, else the platform default, bounded by the maximum (`sagat_domain::quota::resolve` and
-- `ceiling`; the defaults and maxima of `QuotaResource`, kept equal by the carry's property test).
CREATE FUNCTION sagat_step_log_policies(p_tenant uuid, p_namespace uuid)
RETURNS TABLE (attempt_records bigint, attempt_bytes bigint, retention_days integer)
LANGUAGE sql STABLE SECURITY INVOKER SET search_path = pg_catalog, pg_temp AS $$
    WITH resource (name, fallback, maximum) AS (
             VALUES ('step_log_attempt_records', 10000::bigint, 100000::bigint),
                    ('step_log_attempt_bytes', 1048576, 16777216),
                    ('step_log_retention_days', 14, 365)),
         stored AS (
             SELECT r.name,
                    least(coalesce((SELECT c.max_value FROM public.quota_ceilings c
                                     WHERE c.tenant_id = p_tenant AND c.resource = r.name AND c.max_value >= 0),
                                   r.fallback),
                          r.maximum) AS ceiling,
                    coalesce((SELECT l.max_value FROM public.quota_limits l
                               WHERE l.tenant_id = p_tenant AND l.namespace_id = p_namespace AND l.resource = r.name
                                 AND l.max_value >= 0),
                             (SELECT l.max_value FROM public.quota_limits l
                               WHERE l.tenant_id = p_tenant AND l.namespace_id IS NULL AND l.resource = r.name
                                 AND l.max_value >= 0)) AS setting
               FROM resource r)
    SELECT max(least(coalesce(s.setting, s.ceiling), s.ceiling)) FILTER (WHERE s.name = 'step_log_attempt_records'),
           max(least(coalesce(s.setting, s.ceiling), s.ceiling)) FILTER (WHERE s.name = 'step_log_attempt_bytes'),
           (max(least(coalesce(s.setting, s.ceiling), s.ceiling)) FILTER (WHERE s.name = 'step_log_retention_days'))::integer
      FROM stored s
$$;

-- T5 and T6 for a carry (`plan_batch` with a `CarryMark`): the stream's counters, the attempt ceilings, the tenant's
-- remaining allowance (null when the quota is not enforced), the records (sequence, text bytes, truncated), the carry's
-- highest sequence, whether it is in order, and its mark-only reason (null for a full carry; else invalid, secret,
-- content_fallback, failed, tenant_quota or service_busy). A full carry's sequences are strictly increasing (the
-- caller checked them); a mark-only carry's may be in any order, duplicates included. `keep` holds the indices, from
-- 1, of the records to insert, in sequence order; `drops` the sequences this plan adds to the drop counts in the order
-- of `DropReason::ALL` (worker_gap first), before saturation; the counters are those after the plan; `charge` is
-- what the kept rows and the marker will be charged (text bytes plus 128 a row). A full carry runs no statement here.
CREATE FUNCTION sagat_step_log_carry_plan(
    p_records bigint, p_bytes bigint, p_last bigint, p_ceiling_sequence bigint, p_ceiling_reason text,
    p_not_stored_reason text, p_dropped_by_worker bigint, p_dropped_by_server bigint, p_next_marker bigint,
    p_truncated_records bigint, p_max_records bigint, p_max_bytes bigint, p_remaining bigint,
    p_sequences bigint[], p_sizes integer[], p_truncated boolean[], p_highest bigint, p_in_order boolean,
    p_mark text,
    OUT keep integer[], OUT marker_sequence bigint, OUT marker_reason text, OUT records bigint, OUT bytes bigint,
    OUT last_sequence bigint, OUT ceiling_sequence bigint, OUT ceiling_reason text, OUT not_stored_reason text,
    OUT dropped_by_worker bigint, OUT dropped_by_server bigint, OUT next_marker_sequence bigint,
    OUT truncated_records bigint, OUT drops bigint[], OUT charge bigint)
LANGUAGE plpgsql IMMUTABLE SECURITY INVOKER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_reasons constant text[] := ARRAY['worker_gap', 'attempt_records', 'attempt_bytes', 'tenant_quota',
                                       'service_busy', 'out_of_order', 'invalid', 'secret', 'content_fallback',
                                       'failed'];
    v_saturation constant bigint := 9007199254740992;
    v_fresh integer[] := '{}';
    v_sent bigint := 0;
    v_held bigint := 0;
    v_batch_mark bigint := 0;
    v_mark bigint;
    v_gap bigint;
    v_below bigint;
    v_gap_server bigint;
    v_met text := p_ceiling_reason;
    v_first_met text;
    v_kept_bytes bigint := 0;
    v_truncated bigint := 0;
    v_server bigint := 0;
    i integer;
BEGIN
    keep := '{}';
    drops := array_fill(0::bigint, ARRAY[10]);
    charge := 0;
    records := p_records;
    bytes := p_bytes;
    not_stored_reason := p_not_stored_reason;
    -- The fresh records: above the stream's last sequence; for a mark-only carry the distinct ones up to its highest.
    IF p_mark IS NOT NULL THEN
        SELECT count(DISTINCT u.s), coalesce(max(u.s), 0), count(DISTINCT u.s) FILTER (WHERE u.s <= p_ceiling_sequence)
          INTO v_sent, v_batch_mark, v_held
          FROM unnest(p_sequences) AS u(s)
         WHERE u.s > p_last AND u.s <= p_highest;
    ELSE
        FOR i IN 1 .. coalesce(cardinality(p_sequences), 0) LOOP
            IF p_sequences[i] > p_last THEN
                v_fresh := v_fresh || i;
                v_sent := v_sent + 1;
                v_batch_mark := p_sequences[i];
                IF p_sequences[i] <= p_ceiling_sequence THEN
                    v_held := v_held + 1;
                END IF;
            END IF;
        END LOOP;
    END IF;
    -- The gap: below a ceiling already met, the worker's; above it, the ceiling's.
    v_mark := greatest(v_batch_mark, p_highest, p_last);
    v_gap := greatest(v_mark - p_last - v_sent, 0);
    IF p_ceiling_sequence IS NULL THEN
        drops[1] := v_gap;
    ELSE
        v_below := CASE WHEN p_ceiling_sequence > p_last
                        THEN greatest(least(p_ceiling_sequence, v_mark) - p_last - v_held, 0) ELSE 0 END;
        drops[1] := v_below;
        v_gap_server := v_gap - v_below;
        IF v_gap_server > 0 THEN
            i := array_position(v_reasons, coalesce(p_ceiling_reason, 'tenant_quota'));
            drops[i] := drops[i] + v_gap_server;
        END IF;
    END IF;
    IF p_mark IS NOT NULL THEN
        i := array_position(v_reasons, p_mark);
        drops[i] := drops[i] + v_sent;
        IF p_mark = 'tenant_quota' AND v_met IS NULL THEN
            v_met := 'tenant_quota';
            v_first_met := v_met;
        ELSIF p_mark = 'service_busy' THEN
            not_stored_reason := 'service_busy';
        END IF;
    ELSIF NOT p_in_order THEN
        drops[6] := drops[6] + v_sent;
    ELSE
        FOREACH i IN ARRAY v_fresh LOOP
            EXIT WHEN v_met IS NOT NULL;
            IF records + 1 > p_max_records THEN
                v_met := 'attempt_records';
                v_first_met := v_met;
                EXIT;
            END IF;
            IF bytes + p_sizes[i] > p_max_bytes THEN
                v_met := 'attempt_bytes';
                v_first_met := v_met;
                EXIT;
            END IF;
            records := records + 1;
            bytes := bytes + p_sizes[i];
            v_kept_bytes := v_kept_bytes + p_sizes[i];
            IF p_truncated[i] THEN
                v_truncated := v_truncated + 1;
            END IF;
            keep := keep || i;
        END LOOP;
        IF v_first_met IN ('attempt_records', 'attempt_bytes') THEN
            marker_sequence := p_next_marker;
            marker_reason := v_first_met;
        END IF;
        charge := v_kept_bytes + 128 * (cardinality(keep) + CASE WHEN marker_sequence IS NULL THEN 0 ELSE 1 END);
        IF p_remaining IS NOT NULL AND charge > 0 AND charge > p_remaining THEN
            -- All or nothing: what is kept does not fit the tenant's allowance.
            keep := '{}';
            marker_sequence := NULL;
            marker_reason := NULL;
            records := p_records;
            bytes := p_bytes;
            v_truncated := 0;
            charge := 0;
            drops[4] := drops[4] + v_sent;
            IF v_met IS NULL OR v_first_met IS NOT NULL THEN
                v_met := 'tenant_quota';
                v_first_met := v_met;
            END IF;
        ELSIF v_sent > cardinality(keep) THEN
            i := array_position(v_reasons, coalesce(v_met, 'tenant_quota'));
            drops[i] := drops[i] + v_sent - cardinality(keep);
        END IF;
    END IF;
    FOR i IN 2 .. 10 LOOP
        v_server := v_server + drops[i];
    END LOOP;
    last_sequence := v_mark;
    ceiling_sequence := coalesce(p_ceiling_sequence, CASE WHEN v_first_met IS NOT NULL THEN v_mark END);
    ceiling_reason := coalesce(p_ceiling_reason, v_met);
    dropped_by_worker := least(p_dropped_by_worker + drops[1], v_saturation);
    dropped_by_server := least(p_dropped_by_server + v_server, v_saturation);
    next_marker_sequence := p_next_marker + CASE WHEN marker_sequence IS NULL THEN 0 ELSE 1 END;
    truncated_records := p_truncated_records + v_truncated;
END
$$;

-- CC to CE of design section 2a: the attempt's new stream inserted once with what its plan stores (CC), else the
-- existing stream locked if free (CC′) and planned from its counters; the kept lines and the marker (CD, whose
-- statement trigger charges them once); the existing stream's counters and final mark (CE). `outcome` is `written`
-- (a full carry), `mark_only`, `contended` (the stream row held), `removed` (removed or expired) or `final_recorded`.
-- `drops` and `charge` are the plan's (see `sagat_step_log_carry_plan`). With `p_reserve` (inside the block), a plan that
-- charges first reserves its exact charge: `reserved` says whether the writer's quota lease held it, so that the
-- caller tells a charge that found room from one that refilled, and `reserved_room` is that lease's room before CD.
CREATE FUNCTION sagat_step_log_carry_write(
    p_tenant uuid, p_namespace uuid, p_run uuid, p_step uuid, p_node text,
    p_path_nodes text[], p_path_indices integer[], p_attempt integer, p_generation bigint,
    p_sequences bigint[], p_levels text[], p_worker_times timestamptz[], p_messages text[],
    p_exceptions text[], p_fields text[], p_truncated boolean[], p_sizes integer[],
    p_highest bigint, p_mark text, p_max_records bigint, p_max_bytes bigint, p_retention integer,
    p_remaining bigint, p_reserve boolean,
    OUT outcome text, OUT kept integer, OUT drops bigint[], OUT ceiling_reason text, OUT not_stored_reason text,
    OUT charge bigint, OUT reserved boolean, OUT reserved_room bigint)
LANGUAGE plpgsql VOLATILE SECURITY INVOKER SET search_path = pg_catalog, pg_temp AS $$
#variable_conflict use_column
DECLARE
    v_plan record;
    v_row record;
    v_stream bigint;
    v_now timestamptz := clock_timestamp();
    v_rows integer := 0;
    v_planned integer;
    v_existing boolean := false;
BEGIN
    kept := 0;
    charge := 0;
    reserved := false;
    -- The plan of a new stream, before any write (D041-111).
    v_plan := public.sagat_step_log_carry_plan(0, 0, 0, NULL, NULL, NULL, 0, 0, 4611686018427387904, 0,
        p_max_records, p_max_bytes, p_remaining, p_sequences, p_sizes, p_truncated, p_highest, true, p_mark);
    -- CC: the statement's snapshot sees an existing row, so no row is offered and no unique check waits; with the
    -- stream lock held no other transaction is inserting one.
    INSERT INTO public.step_log_streams AS t (tenant_id, public_id, namespace_id, run_id, step_id, node,
            instance_path_nodes, instance_path_indices, attempt, generation, retention_days, records, bytes,
            last_sequence, ceiling_sequence, next_marker_sequence, dropped_by_worker, dropped_by_server,
            ceiling_reason, not_stored_reason, truncated_records, last_stored_xid, final_at, first_at, last_at,
            expires_at)
    SELECT p_tenant, gen_random_uuid(), p_namespace, p_run, p_step, p_node, p_path_nodes, p_path_indices,
           p_attempt, p_generation, p_retention, v_plan.records, v_plan.bytes, v_plan.last_sequence,
           v_plan.ceiling_sequence, v_plan.next_marker_sequence, v_plan.dropped_by_worker,
           v_plan.dropped_by_server, v_plan.ceiling_reason, v_plan.not_stored_reason, v_plan.truncated_records,
           CASE WHEN cardinality(v_plan.keep) > 0 OR v_plan.marker_sequence IS NOT NULL
                THEN pg_current_xact_id() END,
           v_now, v_now, v_now, v_now + make_interval(days => p_retention)
     WHERE NOT EXISTS (SELECT 1 FROM public.step_log_streams e
                        WHERE e.tenant_id = p_tenant AND e.run_id = p_run AND e.step_id = p_step
                          AND e.attempt = p_attempt AND e.generation = p_generation AND e.source = 'worker')
    ON CONFLICT (tenant_id, run_id, step_id, attempt, generation, source) DO NOTHING
    RETURNING t.stream_id INTO v_stream;
    IF v_stream IS NULL THEN
        -- CC′: the existing stream, locked only if free (L2); held by the purge or the delete route: contended.
        SELECT s.stream_id, s.records, s.bytes, s.last_sequence, s.ceiling_sequence, s.ceiling_reason,
               s.not_stored_reason, s.dropped_by_worker, s.dropped_by_server, s.next_marker_sequence,
               s.truncated_records, (s.removed_at IS NOT NULL OR s.expires_at <= clock_timestamp()) AS removed,
               s.final_at IS NOT NULL AS is_final,
               (s.last_stored_xid IS NULL OR s.last_stored_xid < pg_current_xact_id()) AS in_order,
               s.namespace_id
          INTO v_row
          FROM public.step_log_streams s
         WHERE s.tenant_id = p_tenant AND s.run_id = p_run AND s.step_id = p_step AND s.attempt = p_attempt
           AND s.generation = p_generation AND s.source = 'worker'
           FOR UPDATE SKIP LOCKED;
        IF NOT FOUND THEN
            outcome := 'contended';
            RETURN;
        END IF;
        IF v_row.removed THEN
            outcome := 'removed';
            RETURN;
        END IF;
        IF v_row.is_final THEN
            outcome := 'final_recorded';
            RETURN;
        END IF;
        IF v_row.namespace_id IS DISTINCT FROM p_namespace THEN
            -- The attempt's identity is the fence's; a namespace that is not the stream's would take another
            -- namespace's fences and ceilings.
            RAISE EXCEPTION 'a step-log carry names a namespace that is not its stream''s'
                USING ERRCODE = 'invalid_parameter_value';
        END IF;
        v_stream := v_row.stream_id;
        v_existing := true;
        v_plan := public.sagat_step_log_carry_plan(v_row.records, v_row.bytes, v_row.last_sequence,
            v_row.ceiling_sequence, v_row.ceiling_reason, v_row.not_stored_reason, v_row.dropped_by_worker,
            v_row.dropped_by_server, v_row.next_marker_sequence, v_row.truncated_records, p_max_records,
            p_max_bytes, p_remaining, p_sequences, p_sizes, p_truncated, p_highest, v_row.in_order, p_mark);
    END IF;
    -- CD: the kept lines and the marker, at most once each; the statement trigger charges them once.
    v_planned := cardinality(v_plan.keep) + CASE WHEN v_plan.marker_sequence IS NULL THEN 0 ELSE 1 END;
    IF p_reserve AND v_plan.charge > 0 THEN
        SELECT r.reserved, r.room INTO reserved, reserved_room
          FROM public.sagat_quota_step_log_reserve(p_tenant, v_plan.charge) r;
    END IF;
    IF v_planned > 0 THEN
        INSERT INTO public.step_log_lines AS l (tenant_id, stream_id, sequence, run_id, worker_time, level, kind,
                                           truncated, message, exception, fields, marker_reason)
        SELECT p_tenant, v_stream, p_sequences[k.ix], p_run, p_worker_times[k.ix], p_levels[k.ix], 'line',
               p_truncated[k.ix], p_messages[k.ix], p_exceptions[k.ix], p_fields[k.ix], NULL
          FROM unnest(v_plan.keep) AS k(ix)
        UNION ALL
        SELECT p_tenant, v_stream, v_plan.marker_sequence, p_run, NULL, 'info', 'ceiling_marker', false, NULL,
               NULL, NULL, v_plan.marker_reason
         WHERE v_plan.marker_sequence IS NOT NULL
        ON CONFLICT (tenant_id, stream_id, sequence) DO NOTHING;
        GET DIAGNOSTICS v_rows = ROW_COUNT;
        IF v_rows <> v_planned THEN
            -- No row above the stream's last sequence can exist while its stream lock and row are held: the counters
            -- would no longer be exact, so the call fails and the report is re-run without its carry.
            RAISE EXCEPTION 'a step-log carry found a planned line already stored' USING ERRCODE = 'internal_error';
        END IF;
    END IF;
    IF v_existing THEN
        -- CE: the existing stream's counters, its final mark, and the order guard raised only when a row was stored.
        UPDATE public.step_log_streams AS t
           SET records = v_plan.records, bytes = v_plan.bytes,
               last_sequence = greatest(t.last_sequence, v_plan.last_sequence),
               ceiling_sequence = v_plan.ceiling_sequence, ceiling_reason = v_plan.ceiling_reason,
               not_stored_reason = v_plan.not_stored_reason, dropped_by_worker = v_plan.dropped_by_worker,
               dropped_by_server = v_plan.dropped_by_server, next_marker_sequence = v_plan.next_marker_sequence,
               truncated_records = v_plan.truncated_records, first_at = coalesce(t.first_at, v_now),
               last_at = v_now, expires_at = v_now + make_interval(days => t.retention_days::integer),
               final_at = v_now,
               last_stored_xid = CASE WHEN v_rows > 0
                   THEN greatest(coalesce(t.last_stored_xid, '0'::xid8), pg_current_xact_id())
                   ELSE t.last_stored_xid END
         WHERE t.tenant_id = p_tenant AND t.stream_id = v_stream;
    END IF;
    outcome := CASE WHEN p_mark IS NULL THEN 'written' ELSE 'mark_only' END;
    kept := cardinality(v_plan.keep);
    drops := v_plan.drops;
    ceiling_reason := v_plan.ceiling_reason;
    not_stored_reason := v_plan.not_stored_reason;
    charge := v_plan.charge;
END
$$;

-- The carry (data-model.md §8a). Arguments: the attempt the report's fence verified; the records as arrays (for a
-- mark-only carry only the sequences are read); `p_highest`; *Q*, the replica's wait slots; the mark-only reason the
-- application planned (null for a full carry). Returns the outcome (`written`, `mark_only`, `contended`, `off`,
-- `removed`, `final_recorded`), the lines kept, the drops by the worker and by the server per `DropReason`, the
-- stream's `ceiling_reason` and `not_stored_reason`, the quota's cause of a mark-only answer of the block (`limit`,
-- `held`, `busy`), `path` (`fast`, `refill`, `none`), `room` (null exactly when `path` is `none`) and `top_up`.
CREATE FUNCTION sagat_step_log_carry(
    p_tenant uuid, p_namespace uuid, p_run uuid, p_step uuid, p_node text,
    p_path_nodes text[], p_path_indices integer[], p_attempt integer, p_generation bigint,
    p_sequences bigint[], p_levels text[], p_worker_times timestamptz[], p_messages text[],
    p_exceptions text[], p_fields text[], p_truncated boolean[], p_sizes integer[],
    p_highest bigint, p_wait_slots integer, p_mark_reason text)
RETURNS TABLE (outcome text, kept integer, dropped_by_worker bigint, dropped_by_server jsonb, ceiling_reason text,
               not_stored_reason text, cause text, path text, room bigint, top_up boolean)
LANGUAGE plpgsql VOLATILE SECURITY INVOKER SET search_path = pg_catalog, pg_temp AS $$
#variable_conflict use_column
DECLARE
    v_reasons constant text[] := ARRAY['worker_gap', 'attempt_records', 'attempt_bytes', 'tenant_quota',
                                       'service_busy', 'out_of_order', 'invalid', 'secret', 'content_fallback',
                                       'failed'];
    v_n integer := coalesce(cardinality(p_sequences), 0);
    v_previous bigint := 0;
    v_upper bigint := 0;
    v_key text;
    v_max_records bigint;
    v_max_bytes bigint;
    v_retention integer;
    v_used bigint;
    v_limit bigint;
    v_enforced boolean;
    v_remaining bigint;
    v_reserved boolean := false;
    v_reserved_room bigint;
    v_write record;
    v_state text;
    v_constraint text;
    v_mark text := p_mark_reason;
    v_setting text;
    i integer;
BEGIN
    kept := 0;
    dropped_by_worker := 0;
    dropped_by_server := '{}'::jsonb;
    path := 'none';
    top_up := false;
    -- 0. The arguments, before any lock: a fixed text, never a value.
    IF p_mark_reason IS NOT NULL AND p_mark_reason NOT IN ('invalid', 'secret', 'content_fallback', 'failed') THEN
        RAISE EXCEPTION 'a step-log carry takes only the mark-only reasons invalid, secret, content_fallback and failed'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF p_highest IS NULL OR p_highest < 0 OR p_highest > 4611686018427387903 THEN
        RAISE EXCEPTION 'the highest sequence of a step-log carry is outside 0 to 2^62 - 1'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF p_mark_reason IS NULL THEN
        IF coalesce(cardinality(p_levels), 0) <> v_n OR coalesce(cardinality(p_worker_times), 0) <> v_n
           OR coalesce(cardinality(p_messages), 0) <> v_n OR coalesce(cardinality(p_exceptions), 0) <> v_n
           OR coalesce(cardinality(p_fields), 0) <> v_n OR coalesce(cardinality(p_truncated), 0) <> v_n
           OR coalesce(cardinality(p_sizes), 0) <> v_n THEN
            RAISE EXCEPTION 'the arrays of a step-log carry differ in length' USING ERRCODE = 'invalid_parameter_value';
        END IF;
        IF v_n > 64 THEN
            RAISE EXCEPTION 'a step-log carry holds more than 64 records' USING ERRCODE = 'invalid_parameter_value';
        END IF;
        FOR i IN 1 .. v_n LOOP
            IF p_levels[i] IS NULL OR p_levels[i] NOT IN ('trace', 'debug', 'info', 'warn', 'error') THEN
                RAISE EXCEPTION 'a level of a step-log carry is not a level' USING ERRCODE = 'invalid_parameter_value';
            END IF;
            IF p_sequences[i] IS NULL OR p_sequences[i] <= v_previous OR p_sequences[i] > 4611686018427387903 THEN
                RAISE EXCEPTION 'the sequences of a step-log carry are not strictly increasing in 1 to 2^62 - 1'
                    USING ERRCODE = 'invalid_parameter_value';
            END IF;
            -- The size is the text's, as the statement trigger will measure it: the stream's byte counters and the
            -- quota's charge never disagree.
            IF p_sizes[i] IS DISTINCT FROM octet_length(coalesce(p_messages[i], ''))
                   + octet_length(coalesce(p_exceptions[i], '')) + octet_length(coalesce(p_fields[i], '')) THEN
                RAISE EXCEPTION 'a size of a step-log carry is not the size of its text'
                    USING ERRCODE = 'invalid_parameter_value';
            END IF;
            v_previous := p_sequences[i];
            v_upper := v_upper + p_sizes[i] + 128;
        END LOOP;
        IF p_highest < v_previous THEN
            RAISE EXCEPTION 'the highest sequence of a step-log carry is below its last record'
                USING ERRCODE = 'invalid_parameter_value';
        END IF;
        IF v_n > 0 THEN
            -- A possible ceiling marker.
            v_upper := v_upper + 128;
        END IF;
    END IF;
    -- Q for the refill of this transaction's inserts (decision D041-137), transaction-local.
    v_setting := set_config('sagat.step_log_wait_slots', coalesce(p_wait_slots, 1)::text, true);
    -- CA: the six policy fences shared, the stream lock, the tenant's three quota class fences shared: all tried.
    FOREACH v_key IN ARRAY public.sagat_step_log_policy_fence_keys(p_tenant, p_namespace) LOOP
        IF NOT pg_try_advisory_xact_lock_shared(hashtextextended(v_key, 0)) THEN
            outcome := 'contended';
            RETURN NEXT;
            RETURN;
        END IF;
    END LOOP;
    IF NOT pg_try_advisory_xact_lock(hashtextextended(
           public.sagat_step_log_stream_lock_key(p_tenant, p_run, p_step, p_attempt, p_generation), 0)) THEN
        outcome := 'contended';
        RETURN NEXT;
        RETURN;
    END IF;
    FOREACH v_key IN ARRAY ARRAY['quota-policy:tenant_content_bytes:', 'quota-policy:tenant_log_bytes:',
                                 'quota-policy:tenant_step_log_bytes:'] LOOP
        IF NOT pg_try_advisory_xact_lock_shared(hashtextextended(v_key || p_tenant::text, 0)) THEN
            outcome := 'contended';
            RETURN NEXT;
            RETURN;
        END IF;
    END LOOP;
    -- CB: the policies and the tenant's allowance, in a statement whose snapshot begins after CA's locks are held.
    SELECT p.attempt_records, p.attempt_bytes, p.retention_days, a.used, a.max_value, a.enforced
      INTO v_max_records, v_max_bytes, v_retention, v_used, v_limit, v_enforced
      FROM public.sagat_step_log_policies(p_tenant, p_namespace) p,
           public.sagat_quota_step_log_allowance(p_tenant) a;
    IF NOT FOUND THEN
        -- No accounting row: nothing to plan against; the report is re-run without its carry.
        RAISE EXCEPTION 'a step-log carry found no quota accounting row' USING ERRCODE = 'internal_error';
    END IF;
    IF v_max_records = 0 THEN
        outcome := 'off';
        RETURN NEXT;
        RETURN;
    END IF;
    v_remaining := CASE WHEN v_enforced THEN greatest(greatest(v_limit, 0) - greatest(v_used, 0), 0) END;
    -- The reservation (D041-113): room for every sent record and a marker in the writer's quota lease, locked for the
    -- transaction. A carry that can charge nothing (mark-only, or no record) takes neither it nor the block.
    IF v_upper > 0 THEN
        SELECT r.reserved, r.room INTO v_reserved, v_reserved_room
          FROM public.sagat_quota_step_log_reserve(p_tenant, v_upper) r;
    END IF;
    IF v_upper = 0 OR v_reserved THEN
        v_write := public.sagat_step_log_carry_write(p_tenant, p_namespace, p_run, p_step, p_node, p_path_nodes,
            p_path_indices, p_attempt, p_generation, p_sequences, p_levels, p_worker_times, p_messages,
            p_exceptions, p_fields, p_truncated, p_sizes, p_highest, v_mark, v_max_records, v_max_bytes,
            v_retention, v_remaining, false);
        IF v_reserved AND v_write.outcome IN ('written', 'mark_only') THEN
            path := 'fast';
            room := v_reserved_room - v_write.charge;
        END IF;
    ELSE
        -- The block: one subtransaction whose charge refills the writer's quota lease (the usage row tried at once,
        -- then a wait slot and at most b for each lock acquisition). The quota's two refusals and a 55P03 are answered
        -- here with a mark-only final mark; anything else is raised again.
        BEGIN
            v_write := public.sagat_step_log_carry_write(p_tenant, p_namespace, p_run, p_step, p_node,
                p_path_nodes, p_path_indices, p_attempt, p_generation, p_sequences, p_levels, p_worker_times,
                p_messages, p_exceptions, p_fields, p_truncated, p_sizes, p_highest, v_mark, v_max_records,
                v_max_bytes, v_retention, v_remaining, true);
            IF v_write.charge > 0 AND v_write.reserved THEN
                -- The exact charge found room after all (the bound asked for the marker and every sent record): no
                -- refill ran.
                path := 'fast';
                room := v_write.reserved_room - v_write.charge;
            ELSIF v_write.charge > 0 THEN
                path := 'refill';
                -- The writer's room after the charge: its quota lease is this transaction's, so it is found.
                SELECT r.room INTO room FROM public.sagat_quota_step_log_reserve(p_tenant, 1) r;
            END IF;
        EXCEPTION WHEN program_limit_exceeded OR lock_not_available THEN
            GET STACKED DIAGNOSTICS v_state = RETURNED_SQLSTATE, v_constraint = CONSTRAINT_NAME;
            IF v_state = '55P03' THEN
                cause := 'busy';
                v_mark := 'service_busy';
            ELSIF v_constraint = 'tenant_step_log_quota_exceeded' THEN
                cause := 'limit';
                v_mark := 'tenant_quota';
            ELSIF v_constraint = 'tenant_step_log_quota_held' THEN
                cause := 'held';
                v_mark := 'tenant_quota';
            ELSE
                RAISE;
            END IF;
            path := 'none';
            room := NULL;
            -- Mark-only, nothing charged: a new stream inserted, an existing one (its row released by the rollback)
            -- taken again if free.
            v_write := public.sagat_step_log_carry_write(p_tenant, p_namespace, p_run, p_step, p_node,
                p_path_nodes, p_path_indices, p_attempt, p_generation, p_sequences, p_levels, p_worker_times,
                p_messages, p_exceptions, p_fields, p_truncated, p_sizes, p_highest, v_mark, v_max_records,
                v_max_bytes, v_retention, v_remaining, false);
        END;
    END IF;
    outcome := v_write.outcome;
    kept := v_write.kept;
    dropped_by_worker := coalesce((v_write.drops)[1], 0);
    FOR i IN 2 .. 10 LOOP
        IF (v_write.drops)[i] > 0 THEN
            dropped_by_server := dropped_by_server || jsonb_build_object(v_reasons[i], (v_write.drops)[i]);
        END IF;
    END LOOP;
    ceiling_reason := v_write.ceiling_reason;
    not_stored_reason := v_write.not_stored_reason;
    top_up := room IS NOT NULL AND room < public.sagat_quota_step_log_top_up_mark();
    RETURN NEXT;
END
$$;

REVOKE ALL ON FUNCTION sagat_step_log_policy_fence_keys(uuid, uuid),
    sagat_step_log_stream_lock_key(uuid, uuid, uuid, integer, bigint), sagat_step_log_policies(uuid, uuid),
    sagat_step_log_carry_plan(bigint, bigint, bigint, bigint, text, text, bigint, bigint, bigint, bigint, bigint,
                              bigint, bigint, bigint[], integer[], boolean[], bigint, boolean, text),
    sagat_step_log_carry_write(uuid, uuid, uuid, uuid, text, text[], integer[], integer, bigint, bigint[], text[],
                               timestamptz[], text[], text[], text[], boolean[], integer[], bigint, text, bigint,
                               bigint, integer, bigint, boolean),
    sagat_step_log_carry(uuid, uuid, uuid, uuid, text, text[], integer[], integer, bigint, bigint[], text[],
                         timestamptz[], text[], text[], text[], boolean[], integer[], bigint, integer, text)
    FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT EXECUTE ON FUNCTION sagat_step_log_policy_fence_keys(uuid, uuid),
            sagat_step_log_stream_lock_key(uuid, uuid, uuid, integer, bigint), sagat_step_log_policies(uuid, uuid),
            sagat_step_log_carry_plan(bigint, bigint, bigint, bigint, text, text, bigint, bigint, bigint, bigint,
                                      bigint, bigint, bigint, bigint[], integer[], boolean[], bigint, boolean, text),
            sagat_step_log_carry_write(uuid, uuid, uuid, uuid, text, text[], integer[], integer, bigint, bigint[],
                                       text[], timestamptz[], text[], text[], text[], boolean[], integer[], bigint,
                                       text, bigint, bigint, integer, bigint, boolean),
            sagat_step_log_carry(uuid, uuid, uuid, uuid, text, text[], integer[], integer, bigint, bigint[], text[],
                                 timestamptz[], text[], text[], text[], boolean[], integer[], bigint, integer, text)
            TO sagat_app;
    END IF;
END
$grants$;
