-- Creates again the feature step of the removed `control-plane flow-control-activate`, as
-- `20270211000400_flow_control_capabilities` defined it: how many of the required kinds have no live process (seen
-- within 30 seconds) that lists the capability. Operator only.
CREATE FUNCTION flow_control_kinds_without_capable_process(p_capability text, p_kinds text[]) RETURNS bigint
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT count(*) FROM unnest(p_kinds) AS k(kind)
     WHERE NOT EXISTS (SELECT 1 FROM public.serving_instances s
                        WHERE s.kind = k.kind AND s.last_seen > clock_timestamp() - interval '30 seconds'
                          AND p_capability = ANY (s.capabilities));
$$;
REVOKE ALL ON FUNCTION flow_control_kinds_without_capable_process(text, text[]) FROM PUBLIC;
