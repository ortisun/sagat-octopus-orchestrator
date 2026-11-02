"""Dormancy/source inventory checks; these are not a Rust call-graph proof."""
import importlib.util
import pathlib
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "check-capacity-activation.py"
SPEC = importlib.util.spec_from_file_location("capacity_activation", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class CapacityActivation(unittest.TestCase):
    def test_production_cannot_enable_the_incomplete_guard(self):
        errors = MODULE.check_sources({
            "apps/execution-engine/src/main.rs": "workers.with_child_root_capacity_guard()",
        })
        self.assertTrue(errors)
        self.assertIn("activation", errors[0])

    def test_each_native_operation_remains_dormant(self):
        for operation in ("acquire", "vacate", "finish"):
            with self.subTest(operation=operation):
                errors = MODULE.check_sources({
                    "crates/scheduler/src/passes.rs":
                    f"store.{operation}_current_child_root(&command).await",
                })
                self.assertTrue(errors)

    def test_a_new_lease_producer_requires_inventory_review(self):
        errors = MODULE.check_sources({
            "crates/application/src/new_worker.rs": "Some(LeaseGrant { instance })",
        })
        self.assertTrue(errors)
        self.assertIn("lease producer", errors[0])

    def test_current_producers_and_native_method_definitions_are_allowed(self):
        self.assertEqual(MODULE.check_sources({
            "crates/application/src/worker.rs": "Some(LeaseGrant { instance })",
            "crates/application/src/worker/scoped_claims.rs": "Some(LeaseGrant { instance })",
            "crates/infra-postgres/src/capacity_acquire.rs":
            "async fn acquire_current_child_root(&self) {}",
        }), [])

    def test_test_harnesses_may_exercise_opt_in_and_native_operations(self):
        self.assertEqual(MODULE.check_sources({
            "tests/integration/workflow-primitives/probe.rs":
            "workers.with_child_root_capacity_guard(); store.acquire_current_child_root(c);",
        }), [])


if __name__ == "__main__":
    unittest.main()
