"""Current migration oracles are checked without a build, Docker or an old executable."""
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location(
    "upgrade_boundary", Path(__file__).resolve().parents[1] / "check-upgrade-boundary.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class UpgradeBoundary(unittest.TestCase):
    def test_stale_current_boundary_is_refused(self):
        errors = MODULE.check_boundaries({"profile.rs": "assert_eq!(after,20261226000000);"}, 20261231000000)
        self.assertTrue(errors)
        self.assertIn("20261231000000", errors[0])

    def test_missing_or_duplicate_current_oracle_is_refused(self):
        for source in ("", "assert_eq!(before,20261220000000);",
                       "assert_eq!(after,20261231000000);" * 2):
            with self.subTest(source=source):
                self.assertTrue(MODULE.check_boundaries({"profile.rs": source}, 20261231000000))

    def test_current_oracle_accepts_digit_separators_and_does_not_match_before(self):
        source = "assert_eq!(before,20261220000000); assert_eq!(after,20_261_231_000_000);"
        self.assertEqual(MODULE.check_boundaries({"profile.rs": source}, 20261231000000), [])

    def test_comments_and_string_literals_are_not_executable_oracles(self):
        oracle = "assert_eq!(after,20261231000000);"
        for source in (f"// {oracle}", f"/* {oracle} */",
                       f"/* outer /* nested */ {oracle} */",
                       f'let example = "{oracle}";', f'let example = r##"{oracle}"##;',
                       f'let example = br#"{oracle}"#;'):
            with self.subTest(source=source):
                self.assertTrue(MODULE.check_boundaries({"profile.rs": source}, 20261231000000))

    def test_literals_and_nested_comments_do_not_hide_a_real_oracle(self):
        source = '''let url = "https://example.test/*not a comment*/";
            let raw = r###"\" /* still a raw string */"###;
            let quote = '\\"'; let byte = b'\\x22';
            /* outer /* nested */ comment */
            fn phase<'a>(after: i64) { assert_eq!(after,20_261_231_000_000,); }
        '''
        self.assertEqual(MODULE.check_boundaries({"profile.rs": source}, 20261231000000), [])

    def test_every_current_profile_is_checked(self):
        sources = {"good.rs": "assert_eq!(after,20261231000000);",
                   "stale.rs": "assert_eq!(after,20261226000000);"}
        errors = MODULE.check_boundaries(sources, 20261231000000)
        self.assertEqual(len(errors), 1)
        self.assertIn("stale.rs", errors[0])


if __name__ == "__main__":
    unittest.main()
