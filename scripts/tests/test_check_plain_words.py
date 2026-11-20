"""Tests for scripts/check-plain-words.py (run: python3 -m unittest discover scripts/tests).

The rule (docs/constitution.md 4.7.0, Words in full; AGENTS.md rule 24): documents and code comments write words in
full. The check refuses the abbreviations listed in docs/governance/identifier-scheme.md in the lines that a pull
request's own commits add, and the old identifier forms once that page says `Sweep: done`.

The first class feeds single lines to the matcher: what is an abbreviation, and what only looks like one (the false
positives the patterns were tuned against, all taken from this repository's documents). The second builds throwaway git
repositories and checks which lines are read.
"""
import importlib.util
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "check-plain-words.py"
RULE_PATH = "scripts/check-plain-words.py"
SCHEME = "docs/governance/identifier-scheme.md"
GIT_ENV = {"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}
GIT = ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]


def load():
    spec = importlib.util.spec_from_file_location("check_plain_words", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Matcher(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.module = load()

    def tokens(self, line, identifiers=False):
        return [f.token for f in self.module.line_findings(line, identifiers)]

    def test_review_findings_after_a_reviewer_are_abbreviations(self):
        cases = {
            "(architecture review, finding B1)": ["B1"],
            "Review: architecture B1, N8, security N8": ["B1", "N8", "N8"],
            "step left waiting (correctness N5).": ["N5"],
            "It answers finding m7 of the re-review.": ["m7"],
            "security M7, correctness M5, N1, N6, N7": ["M7", "M5", "N1", "N6", "N7"],
            "findings not relayed (architecture N2–N5, security three nits)": ["N2", "N5"],
            "security r1-M1, N6, N9 and re-audit M1": ["r1-M1", "N6", "N9", "M1"],
            "(security r1-M4; consolidated A-N14)": ["r1-M4", "A-N14"],
            "Blocker B2 of the review": ["B2"],
        }
        for line, expected in cases.items():
            with self.subTest(line=line):
                self.assertEqual(self.tokens(line), expected)

    def test_rule_numbers_after_the_word_rule_or_in_a_list_are_abbreviations(self):
        cases = {
            "requires them to be equal (rule R8).": ["R8"],
            "Rule R9 of check-design-docs refuses a tick": ["R9"],
            "the rules R5 and R6 hold": ["R5", "R6"],
            "| R5, R6, R8 | nothing counted |": ["R5", "R6", "R8"],
            "written after the code | R2, R3, R8, R9, R10 |": ["R2", "R3", "R8", "R9", "R10"],
        }
        for line, expected in cases.items():
            with self.subTest(line=line):
                self.assertEqual(self.tokens(line), expected)

    def test_egm_run_findings_slices_mutations_and_ut_are_abbreviations(self):
        cases = {
            "the stage 5 Z3-1 finding of the third run": ["Z3-1"],
            "C3-2 and K3-8 were fixed": ["C3-2", "K3-8"],
            "the slice S5 adds the listing": ["S5"],
            "slices S6a and S6b": ["S6a", "S6b"],
            "the S6c slice starts the probe": ["S6c"],
            "mutation W3 survived": ["W3"],
            "mutants W1, W2 and W4 were killed": ["W1", "W2", "W4"],
            "covered by a UT in the crate": ["UT"],
        }
        for line, expected in cases.items():
            with self.subTest(line=line):
                self.assertEqual(self.tokens(line), expected)

    def test_proper_names_and_labels_that_look_like_abbreviations_are_not(self):
        for line in (
            "implements memory, filesystem, S3-compatible and R2 blob storage.",
            "a plain-http S3 endpoint or token endpoint",
            "choose Iceberg and fill the REST and S3 sections",
            "measured on an Apple M3 Max with 16 cores",
            "milestones: M0=contracts; M1=durable vertical slice; M2=sagas/events; M3=scripts",
            "supports the M4–M5 delivery.",
            "an envelope from namespace N1 pasted into N2",
            "Wake a suspended Run on an event (P1)",
            "Full B1/B2, canonical tasks and broad rules remain separate B0 obligations.",
            "| 42883 | M3, M9, M10 |",
            "Rule 8 of the design-doc check; Blocker 1, Major 2, minor 7; slice 5; mutation 3",
            "the task T022-064 and the gap G708 are identifiers",
            "UTC and UTF-8 are a time zone and an encoding",
            "lane S1c and wire S6a #190",
        ):
            with self.subTest(line=line):
                self.assertEqual(self.tokens(line), [])

    def test_code_quotations_and_link_targets_are_not_judged(self):
        for line in (
            "the check prints `R8 docs/constitution.md: the version differs`",
            'the owner said "architecture B1 is fine" and moved on',
            "the owner said “mutation W3”",
            "see [the design](docs/designs/rule-R8-slice-S5.md)",
            "https://example.org/rule R8",
        ):
            with self.subTest(line=line):
                self.assertEqual(self.tokens(line), [])

    def test_old_identifier_forms_are_refused_only_after_the_sweep(self):
        line = "closes T022-064, FR-022-035, SC-022-012, NFR-010-001, US02, D041-119, G022-015, G708, O041-040, ADR-0097 and OD-2026-11-20-10"
        self.assertEqual(self.tokens(line), [])
        self.assertEqual(self.tokens(line, identifiers=True),
                         ["T022-064", "FR-022-035", "SC-022-012", "NFR-010-001", "US02", "D041-119", "G022-015",
                          "G708", "O041-040", "ADR-0097", "OD-2026-11-20-10"])

    def test_the_new_identifier_forms_and_paths_pass_after_the_sweep(self):
        for line in (
            "closes Task 022-064, Requirement 022-035, User story 022-02 and Architecture decision 0097",
            "see [Architecture decision 0084](../adr/ADR-0084-step-logs.md)",
            "the file docs/adr/ADR-0084-step-logs.md",
            "the cross-cutting Task EGM-3",
            "a TEGM-3 task",
        ):
            with self.subTest(line=line):
                found = self.tokens(line, identifiers=True)
                if "TEGM-3 task" in line:
                    self.assertEqual(found, ["TEGM-3"])
                else:
                    self.assertEqual(found, [])

    def test_every_finding_says_what_to_write_instead(self):
        found = self.module.line_findings("finding m7 and rule R8 and mutation W3 and slice S5 and Z3-1 and UT", False)
        advice = {f.token: f.instead for f in found}
        self.assertEqual(advice["m7"], "minor 7")
        self.assertEqual(advice["R8"], "Rule 8")
        self.assertEqual(advice["W3"], "mutation 3")
        self.assertEqual(advice["S5"], "slice 5")
        self.assertIn("finding 1", advice["Z3-1"])
        self.assertEqual(advice["UT"], "unit test")
        listed = {f.token: f.instead for f in self.module.line_findings("| R4, R5 | killed |", False)}
        self.assertIn("the word R stands for", listed["R4"], "a bare list may label mutants, not rules")
        reviewed = {f.token: f.instead for f in self.module.line_findings("(security r1-M1; consolidated A-N14)", False)}
        self.assertEqual(reviewed["r1-M1"], "Major 1 of review run 1")
        self.assertEqual(reviewed["A-N14"], "Nit 14 of the consolidated review")

    def test_comments_are_read_from_code_and_strings_are_not(self):
        cases = {
            ("a.rs", "let x = 1; // rule R8 holds"): ["R8"],
            ("a.rs", 'let s = "rule R8"; // fine'): [],
            ("a.rs", 'let u = "http://x"; // slice S5'): ["S5"],
            ("a.rs", "/// the mutation W3 is killed"): ["W3"],
            ("a.rs", "#[cfg(test)] // UT"): ["UT"],
            ("a.ts", " * architecture B1 is fixed here"): ["B1"],
            ("a.go", "x := `rule R8` // ok"): [],
            ("a.py", "x = 1  # finding m7"): ["m7"],
            ("a.py", 'x = "# finding m7"'): [],
            ("a.sql", "SELECT 1; -- slice S5"): ["S5"],
            ("a.sql", "SELECT '-- slice S5';"): [],
            ("a.yaml", "key: value # rule R8"): ["R8"],
            ("a.rs", "let rule_r8 = R8;"): [],
            ("a.rs", "let pair = [R5, R6]; // a pair"): [],
            ("a.py", "pair = [R5, R6]"): [],
        }
        for (path, line), expected in cases.items():
            with self.subTest(path=path, line=line):
                text = self.module.comment_text(path, line)
                found = [f.token for f in self.module.line_findings(text, False)] if text else []
                self.assertEqual(found, expected)

    def test_files_in_scope(self):
        scoped = self.module.in_scope
        self.assertEqual(scoped("docs/guide.md"), "prose")
        self.assertEqual(scoped("specs/022-x/spec.md"), "prose")
        self.assertEqual(scoped("crates/domain/src/lib.rs"), "code")
        self.assertEqual(scoped("scripts/x.py"), "code")
        self.assertIsNone(scoped("docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md"))
        self.assertIsNone(scoped("specs/022-x/reviews/2026-11-20/architecture.md"))
        self.assertIsNone(scoped("README.md"), "only docs/ and specs/ Markdown is prose in scope")
        self.assertIsNone(scoped("docs/hero.png"))
        self.assertIsNone(scoped("vendor/x/lib.rs"))
        self.assertIsNone(scoped("frontend/node_modules/x/a.js"))


class Repo:
    def __init__(self, root, with_rule=True, sweep="pending"):
        self.root = root
        self.git("init", "-q", "-b", "develop")
        self.write("docs/guide.md", "# guide\n\nAn old line with rule R8 in it.\n")
        self.write(SCHEME, f"# scheme\n\nSweep: {sweep}\n")
        if with_rule:
            self.write(RULE_PATH, "# the check\n")
        self.commit("base")
        self.git("checkout", "-q", "-b", "feature/x")

    def git(self, *args):
        return subprocess.run([*GIT, *args], cwd=self.root, env={**os.environ, **GIT_ENV},
                              capture_output=True, text=True, check=True).stdout.strip()

    def write(self, rel, text):
        path = pathlib.Path(self.root, rel)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def commit(self, message, files=None):
        for rel, text in (files or {}).items():
            self.write(rel, text)
        self.git("add", "-A")
        self.git("commit", "-q", "--allow-empty", "-m", message)
        return self.git("rev-parse", "HEAD")

    def run(self, *extra, base="develop"):
        args = [sys.executable, str(SCRIPT), "--root", self.root, *extra]
        if "--all" not in extra:
            args += ["--base", base, "--head", "HEAD"]
        result = subprocess.run(args, capture_output=True, text=True)
        return result.returncode, result.stdout + result.stderr


class ChangedLines(unittest.TestCase):
    def repo(self, **options):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        return Repo(tmp.name, **options)

    def test_an_added_line_is_judged_and_an_old_line_is_not(self):
        repo = self.repo()
        repo.commit("docs: more", {"docs/guide.md": "# guide\n\nAn old line with rule R8 in it.\n\nNew: finding m7.\n"})
        code, out = repo.run()
        self.assertEqual(code, 1, out)
        self.assertIn("docs/guide.md:5", out)
        self.assertIn("m7", out)
        self.assertIn("minor 7", out)
        self.assertNotIn("R8", out, "a line the pull request did not add is not judged")

    def test_a_line_added_then_written_out_passes(self):
        repo = self.repo()
        repo.commit("docs: a", {"docs/guide.md": "# guide\n\nAn old line with rule R8 in it.\n\nfinding m7\n"})
        repo.commit("docs: b", {"docs/guide.md": "# guide\n\nAn old line with rule R8 in it.\n\nminor 7\n"})
        code, out = repo.run()
        self.assertEqual(code, 0, out)

    def test_a_line_inside_a_fence_is_not_judged(self):
        repo = self.repo()
        repo.commit("docs: a", {"specs/001-x/spec.md": "# x\n\n```text\nR8 docs/x.md: rule R8 failed\n```\n"})
        code, out = repo.run()
        self.assertEqual(code, 0, out)

    def test_a_comment_added_in_code_is_judged(self):
        repo = self.repo()
        repo.commit("feat: a", {"crates/a/src/lib.rs": "// slice S5 starts here\npub fn a() {}\n"})
        code, out = repo.run()
        self.assertEqual(code, 1, out)
        self.assertIn("crates/a/src/lib.rs:1", out)

    def test_a_block_comment_line_is_judged(self):
        repo = self.repo()
        repo.commit("feat: a", {"sdks/ts/a.ts": "/*\n  the architecture B1 fix\n*/\nexport const a = 1;\n"})
        code, out = repo.run()
        self.assertEqual(code, 1, out)
        self.assertIn("sdks/ts/a.ts:2", out)

    def test_code_after_a_block_comment_is_not_a_comment(self):
        repo = self.repo()
        repo.commit("feat: a", {"sdks/ts/a.ts": "/*\n  a block\n*/\nexport const pair = [R5, R6];\n// slice S5\n"})
        code, out = repo.run()
        self.assertEqual(code, 1, out)
        self.assertIn("sdks/ts/a.ts:5", out)
        self.assertNotIn("sdks/ts/a.ts:4", out)

    def test_a_review_report_is_not_judged(self):
        repo = self.repo()
        repo.commit("docs: review", {"docs/designs/044-x.reviews/2026-11-20/architecture.md": "B1: architecture B1\n"})
        code, out = repo.run()
        self.assertEqual(code, 0, out)

    def test_lines_of_commits_written_before_the_check_are_not_judged(self):
        repo = self.repo(with_rule=False)
        repo.commit("docs: old", {"docs/old.md": "finding m7\n"})
        repo.git("checkout", "-q", "develop")
        repo.commit("ci: the check", {RULE_PATH: "# the check\n"})
        repo.git("checkout", "-q", "feature/x")
        repo.git("merge", "-q", "--no-edit", "develop")
        code, out = repo.run()
        self.assertEqual(code, 0, out)
        repo.commit("docs: new", {"docs/new.md": "mutation W3\n"})
        code, out = repo.run()
        self.assertEqual(code, 1, out)
        self.assertIn("docs/new.md:1", out)
        self.assertNotIn("docs/old.md", out)

    def test_old_identifiers_follow_the_sweep_line_of_the_scheme(self):
        pending = self.repo(sweep="pending")
        pending.commit("docs: a", {"docs/a.md": "closes T022-064\n"})
        self.assertEqual(pending.run()[0], 0)
        done = self.repo(sweep="done")
        done.commit("docs: a", {"docs/a.md": "closes T022-064\n"})
        code, out = done.run()
        self.assertEqual(code, 1, out)
        self.assertIn("Task 022-064", out)

    def test_all_reads_every_line(self):
        repo = self.repo()
        code, out = repo.run("--all")
        self.assertEqual(code, 1, out)
        self.assertIn("docs/guide.md:3", out)

    def test_an_unknown_base_is_an_error_not_a_pass(self):
        repo = self.repo()
        code, out = repo.run(base="no-such-ref")
        self.assertEqual(code, 2, out)


if __name__ == "__main__":
    unittest.main()
