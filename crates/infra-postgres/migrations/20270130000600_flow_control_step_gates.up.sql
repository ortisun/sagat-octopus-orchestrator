-- Feature 022 (T022-096; FR-022-051; D022-020, D022-044): the readiness gate's projection on `steps`. A step of a node
-- that declares `concurrency` or `rateLimit` waits for its gates as `WAITING` with `waiting_kind` `RATE_LIMIT` or
-- `CONCURRENCY` (the first gate it still waits for, rate first), and `flow_control_gates` holds the state of each gate of
-- its attempts as the reducer left it (phase, attempt, pool, group, the key's 12-hex label and a wait's deadline; never a
-- key's value), so that a transaction restores the step as a replay of its journal would.
--
-- Additive: one nullable column (no rewrite) and three CHECKs added `NOT VALID` (validated by
-- `20270130000700_flow_control_validate`, D043-145). No row has the new values until a run of a gated definition is
-- admitted, which no process of this build lists the capability for (D022-037).
ALTER TABLE steps ADD COLUMN flow_control_gates jsonb;
ALTER TABLE steps DROP CONSTRAINT steps_waiting_kind_valid;
ALTER TABLE steps ADD CONSTRAINT steps_waiting_kind_valid CHECK (
    waiting_kind IS NULL OR waiting_kind IN
    ('TIMER', 'EVENT', 'BRANCHES', 'CHILD', 'SAGA', 'RECONCILIATION', 'OPERATOR', 'COLLECTION', 'LOOP', 'APPROVAL',
     'CONCURRENCY', 'RATE_LIMIT')) NOT VALID;
ALTER TABLE steps ADD CONSTRAINT steps_flow_control_gates_bounded CHECK (
    flow_control_gates IS NULL
    OR (jsonb_typeof(flow_control_gates) = 'object' AND octet_length(flow_control_gates::text) <= 1024)) NOT VALID;
-- a step waiting for its gates has their state
ALTER TABLE steps ADD CONSTRAINT steps_gate_wait_projected CHECK (
    waiting_kind IS NULL OR waiting_kind NOT IN ('CONCURRENCY', 'RATE_LIMIT') OR flow_control_gates IS NOT NULL) NOT VALID;
