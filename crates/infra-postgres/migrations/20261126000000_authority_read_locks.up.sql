-- FR-017-008/019: PostgreSQL SELECT ... FOR SHARE requires UPDATE privilege. The runtime
-- deliberately cannot change memberships or namespace identity. Narrow read/lock functions let
-- admission fence operator revocation without granting UPDATE or returning protected values.
CREATE FUNCTION public.sagat_lock_memberships(p_tenant uuid, p_subjects text[])
RETURNS TABLE(subject text, role text)
LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT m.subject, m.role FROM public.memberships m
    WHERE m.tenant_id=p_tenant AND m.subject=ANY(p_subjects)
    ORDER BY m.subject FOR SHARE OF m
$$;
CREATE FUNCTION public.sagat_lock_namespace(p_tenant uuid, p_namespace uuid)
RETURNS TABLE(environment text)
LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT n.environment FROM public.namespaces n
    WHERE n.tenant_id=p_tenant AND n.id=p_namespace FOR SHARE OF n
$$;
CREATE FUNCTION public.sagat_lock_secret_access(p_tenant uuid, p_secret uuid, p_subjects text[])
RETURNS TABLE(subject text)
LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT a.subject FROM public.secret_access a
    WHERE a.tenant_id=p_tenant AND a.secret_id=p_secret AND a.subject=ANY(p_subjects)
    ORDER BY a.subject FOR SHARE OF a
$$;
REVOKE ALL ON FUNCTION public.sagat_lock_memberships(uuid,text[]) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.sagat_lock_namespace(uuid,uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.sagat_lock_secret_access(uuid,uuid,text[]) FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='sagat_app') THEN
        GRANT EXECUTE ON FUNCTION public.sagat_lock_memberships(uuid,text[]),
            public.sagat_lock_namespace(uuid,uuid), public.sagat_lock_secret_access(uuid,uuid,text[])
            TO sagat_app;
    END IF;
END
$grants$;
