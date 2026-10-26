-- Restores the previous behaviour: a grant survives a change of role (and is inert while the role
-- lacks payload:read-internal). No data is lost: the trigger only ever deleted rows.
DROP TRIGGER IF EXISTS memberships_role_change_revokes_reveal_grants ON public.memberships;
DROP FUNCTION IF EXISTS public.sagat_revoke_reveal_grants_on_role_change();
