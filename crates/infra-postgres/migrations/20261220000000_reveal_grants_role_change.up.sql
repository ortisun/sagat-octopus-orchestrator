-- A reveal grant belongs to the role the subject held when an administrator granted it (feature 016,
-- D275, D660). Demoting a member below `payload:read-internal` made the grant inert but left the
-- row, so promoting the member again brought back a right nobody had renewed. Any change of the
-- role of a membership now removes that subject's grants in every namespace of the tenant, as
-- removing the membership already does (ON DELETE CASCADE); an administrator grants again.
--
-- SECURITY DEFINER because the role that changes memberships has no privilege on the grants table
-- and the runtime role cannot change memberships at all. The function pins its search_path and is
-- not executable by anyone: only the trigger runs it.

CREATE FUNCTION public.sagat_revoke_reveal_grants_on_role_change() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    DELETE FROM public.payload_reveal_grants
     WHERE tenant_id = OLD.tenant_id AND subject = OLD.subject;
    RETURN NEW;
END
$$;

REVOKE ALL ON FUNCTION public.sagat_revoke_reveal_grants_on_role_change() FROM PUBLIC;

CREATE TRIGGER memberships_role_change_revokes_reveal_grants
    AFTER UPDATE OF role ON public.memberships
    FOR EACH ROW WHEN (OLD.role IS DISTINCT FROM NEW.role)
    EXECUTE FUNCTION public.sagat_revoke_reveal_grants_on_role_change();
ALTER TABLE public.memberships
    ENABLE ALWAYS TRIGGER memberships_role_change_revokes_reveal_grants;
