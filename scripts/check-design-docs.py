#!/usr/bin/env python3
"""Check the design docs under docs/designs (rules R1-R9 of docs/designs/constitution-4.0.0-egm.md).

Usage: python3 scripts/check-design-docs.py [--root DIR]

It reads docs/designs/*.md (except README.md and TEMPLATE.md), docs/owner-decisions.md,
docs/constitution.md and docs/engineering/egm-accepted-hashes.json, never edits a file, prints one
line per failure as "<rule> <path>[:<line>]: <message>" (never file content), and fails rather than
skips a rule it cannot run. GITHUB_EVENT_NAME=pull_request makes the run strict; GITHUB_BASE_REF
names the base branch (origin/develop otherwise).

Exit codes: 0 no failure; 1 at least one failure; 2 not a git repository.
"""
import argparse
import glob
import hashlib
import json
import os
import re
import subprocess
import sys

FENCE_RE = re.compile(r"^ {0,3}(```(?=[^`]*$)|~~~)")
HYPHEN_RE = re.compile(r"^-{20,}$")
FIELD_RE = re.compile(r"^\*   \*\*([A-Za-z][A-Za-z -]*):\*\* (.*)$")
TASK_RE = re.compile(r"^\s*[-*] \[( |x|X)\] (T[0-9]{3}-[0-9]+|T[A-Z]{2,8}-[0-9]+) ")
MARKER_RE = re.compile(r"^\s+Evidence \([0-9]{4}-[0-9]{2}-[0-9]{2}\):")
INLINE_MARKER_RE = re.compile(r"Evidence \([0-9]{4}-[0-9]{2}-[0-9]{2}\):")
PROTECTED_RE = re.compile(r"^\s*(Refs|Owner-gate):")
OD_ID_RE = re.compile(r"OD-[0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9]+")
COUNTS_RE = re.compile(r"Counts: Blocker ([0-9]{1,4}), Major ([0-9]{1,4}), Nit ([0-9]{1,4})")
SEMVER_RE = re.compile(r"[0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}")
LOOSE_CHECKBOX_RE = re.compile(r"^\s*[-*+] \[[ xX]\]")
SAFE_SHOWN = re.compile(r"[A-Za-z0-9._/#:+-]{1,80}")
HEX40 = re.compile(r"[0-9a-f]{40}")
HEX64 = re.compile(r"[0-9a-f]{64}")
SAFE_REF = re.compile(r"[A-Za-z0-9._/-]{1,80}")
LOCAL_PATH_RE = re.compile(r"(/Users/|/home/|[A-Za-z]:\\)")
STATUSES = ("Draft", "Proposed", "In Review", "Approved")
CHANGEABLE_FIELDS = {"Status", "Date", "Reviewed-commit", "EGM-hash", "Claude-Code", "Reports",
                     "Open findings", "Accepted"}
APPROVAL_FIELDS = ("Reviewed-commit", "EGM-hash", "Claude-Code", "Reports", "Open findings", "Accepted")
REVIEWERS = ("egm-review", "architecture", "security", "correctness")
ROMAN = ("I", "II", "III", "IV", "V", "VI", "VII", "VIII")
OD_COLUMNS = {"ID", "Decision", "Decided by", "Status", "Covers"}
FOLLOW_UP = "#### Follow-up tasks"
MAX_EVIDENCE_LINES = 40


def fence_flags(lines):
    """True for every line that is a fence line or lies between two fence lines."""
    flags, marker = [], None
    for line in lines:
        match = FENCE_RE.match(line)
        if marker is None:
            marker = match.group(1) if match else None
            flags.append(bool(match))
        else:
            flags.append(True)
            if match and match.group(1) == marker:
                marker = None
    return flags


def shown(value):
    """A value from a doc, safe to print: plain and short, otherwise a placeholder."""
    return value if SAFE_SHOWN.fullmatch(value) else "(malformed)"


def header_end(lines):
    for index, line in enumerate(lines):
        if HYPHEN_RE.match(line):
            return index
    return None


class Task:
    def __init__(self, task_id, checked, start, end, marker):
        self.id, self.checked, self.start, self.end, self.marker = task_id, checked, start, end, marker


def parse_tasks(lines, flags, begin):
    tasks, index = [], begin
    while index < len(lines):
        match = None if flags[index] else TASK_RE.match(lines[index])
        if not match:
            index += 1
            continue
        indent = len(lines[index]) - len(lines[index].lstrip())
        probe = index + 1
        while probe < len(lines) and (lines[probe].strip() == "" or lines[probe].startswith("  ")):
            sibling = None if flags[probe] else TASK_RE.match(lines[probe])
            if sibling and len(lines[probe]) - len(lines[probe].lstrip()) <= indent:
                break
            probe += 1
        end = probe - 1
        while end > index and lines[end].strip() == "":
            end -= 1
        marker = next((k for k in range(index + 1, end + 1) if MARKER_RE.match(lines[k])), None)
        tasks.append(Task(match.group(2), match.group(1) in ("x", "X"), index, end, marker))
        index = end + 1
    return tasks


class Doc:
    def __init__(self, rel, text):
        self.rel, self.text = rel, text
        self.lines = text.split("\n")
        self.flags = fence_flags(self.lines)
        self.hend = header_end(self.lines)
        self.stem = os.path.basename(rel)[:-3]
        match = re.fullmatch(r"([0-9]{3})-(.+)", self.stem)
        self.number, self.slug = (match.group(1), match.group(2)) if match else (None, self.stem)
        self.fields, self.duplicate_fields = {}, []
        for index in range(self.hend if self.hend is not None else 0):
            field = FIELD_RE.match(self.lines[index])
            if field:
                if field.group(1) in self.fields:
                    self.duplicate_fields.append(field.group(1))
                self.fields[field.group(1)] = field.group(2)
        begin = self.hend + 1 if self.hend is not None else 0
        self.tasks = parse_tasks(self.lines, self.flags, begin)
        covered = {k for task in self.tasks for k in range(task.start, task.end + 1)}
        self.stray_checkboxes = [i for i in range(begin, len(self.lines))
                                 if i not in covered and not self.flags[i] and LOOSE_CHECKBOX_RE.match(self.lines[i])]

    def sections(self):
        """Map heading text of each level-2 heading outside fences to its content lines."""
        found, current, in_comment = {}, None, False
        for line, fenced in zip(self.lines, self.flags):
            if fenced:
                continue
            if in_comment:
                in_comment = "-->" not in line
                continue
            line = re.sub(r"<!--.*?-->", "", line)
            if "<!--" in line:
                in_comment = True
                continue
            if line.startswith("## "):
                current = line[3:].strip()
                found[current] = []
            elif current is not None:
                found[current].append(line)
        return found


def normalized_body(doc, keep_ids):
    """The body of a doc with the parts that may change after approval removed (R3, step 2)."""
    lines, flags = doc.lines, doc.flags
    by_start = {task.start: task for task in doc.tasks}
    out, in_region, index = [], False, doc.hend + 1
    while index < len(lines):
        line = lines[index]
        if not flags[index]:
            if line.strip() == FOLLOW_UP:
                in_region = True
                out.append((index + 1, line))
                index += 1
                continue
            if in_region and re.match(r"^#{1,4} ", line):
                in_region = False
        task = by_start.get(index)
        if task is not None:
            if not (in_region and keep_ids is not None and task.id not in keep_ids):
                out.append((index + 1, re.sub(r"^(\s*[-*] )\[[ xX]\]", r"\1[ ]", lines[task.start])))
                stop = task.marker if task.marker is not None else task.end + 1
                out.extend((k + 1, lines[k]) for k in range(task.start + 1, stop) if lines[k].strip())
            index = task.end + 1
            continue
        if not flags[index] and (line.strip() == "" or (in_region and line.strip() == "None yet.")):
            index += 1
            continue
        out.append((index + 1, line))
        index += 1
    return out


def header_core(doc):
    out = []
    for index in range(doc.hend + 1):
        line = doc.lines[index]
        field = FIELD_RE.match(line)
        if line.strip() and not (field and field.group(1) in CHANGEABLE_FIELDS):
            out.append((index + 1, line))
    return out


def first_difference(old, new, fallback_line):
    for index in range(max(len(old), len(new))):
        if index >= len(new):
            return fallback_line
        if index >= len(old) or old[index][1] != new[index][1]:
            return new[index][0]
    return None


def principle_blocks(text):
    lines = text.replace("\r\n", "\n").split("\n")
    blocks = {}
    for start, line in enumerate(lines):
        match = re.match(r"^### (I|II|III|IV|V|VI|VII|VIII)\. ", line)
        if not match:
            continue
        end = next((k for k in range(start + 1, len(lines)) if lines[k].startswith(("## ", "### "))), len(lines))
        block = lines[start:end]
        while block and not block[-1].strip():
            block.pop()
        blocks[match.group(1)] = hashlib.sha256("\n".join(block).encode("utf-8")).hexdigest()
    return blocks


def split_row(line):
    return [cell.strip() for cell in line.strip().strip("|").split("|")]


class Checker:
    def __init__(self, root):
        self.root = os.path.realpath(root)
        self.failures, self.warnings = [], []
        event = os.environ.get("GITHUB_EVENT_NAME", "")
        self.strict = event == "pull_request"
        base = os.environ.get("GITHUB_BASE_REF", "") if self.strict else ""
        if base and not SAFE_REF.fullmatch(base):
            self.fail("R9", ".", None, "GITHUB_BASE_REF is not a plain branch name")
            base = ""
        self.base = f"origin/{base or 'develop'}"
        self._base_ok = None
        self._hashes = None
        self._od = None
        self.agent_blobs = {}

    # -- helpers ---------------------------------------------------------------------------
    def fail(self, rule, rel, line, message):
        where = f"{rel}:{line}" if line else rel
        self.failures.append(f"{rule} {where}: {message}")

    def git(self, *args):
        result = subprocess.run(["git", "-C", self.root, *args], capture_output=True)
        return result.returncode, result.stdout.decode("utf-8", errors="replace")

    def read(self, rel):
        try:
            with open(os.path.join(self.root, rel), encoding="utf-8", newline="") as handle:
                return handle.read()
        except FileNotFoundError:
            return None
        except (OSError, UnicodeDecodeError):
            self.fail("R1", rel, None, "the file cannot be read as UTF-8")
            return None

    def base_ok(self):
        if self._base_ok is None:
            self._base_ok = self.git("rev-parse", "--verify", "--quiet", f"{self.base}^{{commit}}")[0] == 0
            if not self._base_ok:
                self.fail("R9", ".", None, f"base ref {self.base} not found: fetch it (fetch-depth 0)")
        return self._base_ok

    def base_text(self, rel):
        """Text of the doc at the base ref; None when the doc is new or the base is missing."""
        if not self.base_ok():
            return None
        code, out = self.git("show", f"{self.base}:{rel}")
        return out if code == 0 else None

    def accepted_hashes(self):
        if self._hashes is None:
            self._hashes = self._load_hashes()
        return self._hashes

    def _load_hashes(self):
        rel = "docs/engineering/egm-accepted-hashes.json"
        text = self.read(rel)
        if text is None:
            return None, f"{rel} is missing"
        try:
            data = json.loads(text)
            minimum = data["minimum_claude_code"]
            if not isinstance(minimum, str) or not SEMVER_RE.fullmatch(minimum):
                return None, f"{rel}: minimum_claude_code is not N.N.N"
            hashes = set()
            for entry in data["entries"]:
                if not (HEX64.fullmatch(entry["hash"]) and HEX40.fullmatch(entry["commit"])
                        and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", entry["accepted"])):
                    return None, f"{rel}: an entry is malformed"
                hashes.add(entry["hash"])
        except (ValueError, KeyError, TypeError, RecursionError):
            return None, f"{rel} cannot be parsed"
        return (hashes, tuple(int(p) for p in minimum.split("."))), None

    # -- owner decisions -------------------------------------------------------------------
    def owner_rows(self):
        if self._od is None:
            self._od = self._parse_owner_decisions()
        return self._od

    def _parse_owner_decisions(self):
        rel = "docs/owner-decisions.md"
        text = self.read(rel)
        rows = []
        if text is None:
            return rows
        lines = text.split("\n")
        flags = fence_flags(lines)
        header, in_comment = None, False
        for number, line in enumerate(lines, start=1):
            hidden = flags[number - 1] or in_comment or line.startswith(("    ", "\t"))
            if "<!--" in line and "-->" not in line.split("<!--", 1)[1]:
                in_comment = True
                hidden = True
            elif in_comment and "-->" in line:
                in_comment = False
                hidden = True
            if hidden:
                if line.strip().startswith("| OD-"):
                    self.fail("R5", rel, number, "hidden row (inside a fence, a comment or indented code)")
                continue
            if not line.startswith("|"):
                header = None
                continue
            cells = split_row(line)
            if header is None:
                header = cells if set(cells) == OD_COLUMNS and len(cells) == len(OD_COLUMNS) else False
                continue
            if header is False or all(re.fullmatch(r":?-+:?", cell) for cell in cells):
                continue
            if "\\|" in line:
                self.fail("R5", rel, number, "an escaped pipe in a row is not supported")
                continue
            if len(cells) != len(header):
                self.fail("R5", rel, number, "the row has a different number of cells than the header")
                continue
            rows.append(dict(zip(header, cells)))
        return rows

    def citable(self, row_id):
        """The citable row with this id, or None."""
        matches = [r for r in self.owner_rows() if r["ID"] == row_id]
        if len(matches) != 1 or not OD_ID_RE.fullmatch(row_id):
            return None
        row = matches[0]
        return row if row["Decided by"] == "owner" and row["Status"] == "Decided" else None

    # -- the run ---------------------------------------------------------------------------
    def run(self):
        docs = []
        designs = os.path.join(self.root, "docs", "designs")
        names = sorted(n for n in os.listdir(designs) if n.endswith(".md")) if os.path.isdir(designs) else []
        for name in names:
            path = os.path.join(designs, name)
            rel = os.path.relpath(path, self.root).replace(os.sep, "/")
            if name in ("README.md", "TEMPLATE.md") or not os.path.isfile(path):
                continue
            text = self.read(rel)
            if text is None:
                self.fail("R1", rel, None, "the file cannot be read as UTF-8")
                continue
            docs.append(Doc(rel, text))
        self.owner_rows()
        known, problem = self.accepted_hashes()
        if known is None:
            self.fail("R2", "docs/engineering/egm-accepted-hashes.json", None, problem)
        for doc in docs:
            self.check_sections(doc)
            self.check_status(doc)
            if "\r" in doc.text:
                continue
            self.check_ids(doc)
            self.check_evidence(doc)
            self.check_gates(doc)
            if doc.fields.get("Status") == "Approved":
                self.check_approved(doc)
        self.check_cross_doc(docs)
        self.check_numbers(docs)
        self.check_constitution()

    # R1 ---------------------------------------------------------------------------------
    def check_sections(self, doc):
        sections = doc.sections()
        for name in ("Constitution Check", "Complexity Tracking"):
            content = sections.get(name)
            if content is None or not any(line.strip() for line in content):
                self.fail("R1", doc.rel, None, f"the section '## {name}' is missing or empty")
        rows = [line for line in sections.get("Constitution Check", [])]
        for prefix in [f"| {r}." for r in ROMAN] + ["| Delivery Constraints"]:
            if not any(line.startswith(prefix) for line in rows):
                self.fail("R1", doc.rel, None, f"the Constitution Check has no row starting '{prefix}'")

    # R9 (status) ------------------------------------------------------------------------
    def check_status(self, doc):
        if "\r" in doc.text:
            self.fail("R9", doc.rel, None, "carriage returns are not allowed: use LF line endings")
            return
        if doc.hend is None:
            self.fail("R9", doc.rel, None, "the header has no line of 20 or more hyphens")
            return
        for field in doc.duplicate_fields:
            self.fail("R9", doc.rel, None, f"the header field {field} appears more than once")
        status = doc.fields.get("Status")
        if status not in STATUSES:
            self.fail("R9", doc.rel, None, "Status is missing or not one of Draft, Proposed, In Review, Approved")
        ticked = [t for t in doc.tasks if t.checked]
        if ticked and status != "Approved":
            old = self.base_text(doc.rel)
            previous = set()
            if old is not None:
                before = Doc(doc.rel, old)
                previous = {t.id for t in before.tasks if t.checked}
            for task in ticked:
                if task.id not in previous:
                    self.fail("R9", doc.rel, task.start + 1, f"{task.id} is newly ticked but the doc is not Approved")

    # R4 ---------------------------------------------------------------------------------
    def check_ids(self, doc):
        if doc.number is not None:
            if "Task-prefix" in doc.fields:
                self.fail("R4", doc.rel, None, "a numbered doc must not declare a Task-prefix")
            pattern = re.compile(rf"T{doc.number}-[0-9]+")
        else:
            prefix = doc.fields.get("Task-prefix", "")
            if not re.fullmatch(r"[A-Z]{2,8}", prefix):
                self.fail("R4", doc.rel, None, "an unnumbered doc needs a Task-prefix of 2 to 8 capital letters")
                return
            pattern = re.compile(rf"T{prefix}-[0-9]+")
        for index in doc.stray_checkboxes:
            self.fail("R4", doc.rel, index + 1, "a checkbox line that is not a task: write '- [ ] T<id> text'")
        seen = set()
        for task in doc.tasks:
            if not pattern.fullmatch(task.id):
                self.fail("R4", doc.rel, task.start + 1, f"{task.id} does not use this doc's number or Task-prefix")
            if task.id in seen:
                self.fail("R4", doc.rel, task.start + 1, f"{task.id} is defined twice")
            seen.add(task.id)

    # R7 ---------------------------------------------------------------------------------
    def check_evidence(self, doc):
        for task in doc.tasks:
            if not task.checked:
                continue
            at = task.start + 1
            if task.marker is None:
                note = " (the marker is on the task line)" if INLINE_MARKER_RE.search(doc.lines[task.start]) else ""
                self.fail("R7", doc.rel, at, f"{task.id} is ticked without an evidence block{note}")
                continue
            block = doc.lines[task.marker:task.end + 1]
            after_colon = block[0].split("):", 1)[1].strip()
            if not after_colon and not any(line.strip() for line in block[1:]):
                self.fail("R7", doc.rel, task.marker + 1, f"{task.id} has an empty evidence block")
            if len(block) > MAX_EVIDENCE_LINES:
                self.fail("R7", doc.rel, task.marker + 1, f"{task.id} has an evidence block of more than 40 lines")
            for offset, line in enumerate(block):
                stripped = line.strip()
                if stripped.startswith(("#", "|", "```", "~~~")):
                    self.fail("R7", doc.rel, task.marker + offset + 1, "an evidence line starts with #, | or a fence")
                if PROTECTED_RE.match(line):
                    self.fail("R7", doc.rel, task.marker + offset + 1, "a protected line sits inside an evidence block")

    # R5 ---------------------------------------------------------------------------------
    def check_gates(self, doc):
        for task in doc.tasks:
            if not task.checked:
                continue
            stop = task.marker if task.marker is not None else task.end + 1
            for index in range(task.start + 1, stop):
                line = doc.lines[index].strip()
                if not line.startswith("Owner-gate:"):
                    continue
                for gate in [g.strip() for g in line[len("Owner-gate:"):].split(",")]:
                    row = self.citable(gate)
                    token = re.compile(rf"(?<![A-Za-z0-9-]){re.escape(task.id)}(?![0-9])")
                    if row is None or not token.search(row["Covers"]):
                        self.fail("R5", doc.rel, index + 1,
                                  f"{task.id} is ticked but {shown(gate) if gate else '(empty)'} is not an owner row that covers it")

    # R2 and R3 --------------------------------------------------------------------------
    def check_approved(self, doc):
        fields = doc.fields
        missing = [f for f in APPROVAL_FIELDS if f not in fields]
        for field in missing:
            self.fail("R2", doc.rel, None, f"the approval field {field} is missing")
        if missing:
            return
        ok = True
        if not fields.get("Author", "").strip():
            self.fail("R2", doc.rel, None, "Author is missing or empty")
        if not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", fields.get("Date", "")):
            self.fail("R2", doc.rel, None, "Date is missing or not YYYY-MM-DD")
        commit = fields["Reviewed-commit"]
        if not HEX40.fullmatch(commit):
            self.fail("R2", doc.rel, None, "Reviewed-commit is not 40 lowercase hex characters")
            ok = False
        egm_hash = fields["EGM-hash"]
        if not HEX64.fullmatch(egm_hash):
            self.fail("R2", doc.rel, None, "EGM-hash is not 64 lowercase hex characters")
            ok = False
        accepted, _ = self.accepted_hashes()
        if accepted is not None:
            known, minimum = accepted
            if HEX64.fullmatch(egm_hash) and egm_hash not in known:
                self.fail("R2", doc.rel, None, "EGM-hash is not on the accepted list")
            version = fields["Claude-Code"]
            if not SEMVER_RE.fullmatch(version):
                self.fail("R2", doc.rel, None, "Claude-Code is not N.N.N")
            elif tuple(int(p) for p in version.split(".")) < minimum:
                self.fail("R2", doc.rel, None, "Claude-Code is older than the accepted minimum")
        if fields["Open findings"] != "Blocker 0, Major 0":
            self.fail("R2", doc.rel, None, "Open findings must read exactly 'Blocker 0, Major 0'")
        count, ids = self.parse_accepted(doc)
        reports = self.parse_reports(doc)
        findings = None
        if reports is not None:
            findings = self.check_reports(doc, reports)
        if findings is not None and count is not None and findings > count:
            self.fail("R2", doc.rel, None, f"the reports hold {findings} Blocker and Major findings, {count} accepted")
        if ids and reports is not None:
            self.check_acceptances(doc, ids, [path for path, _ in reports])
        if ok:
            self.check_commit_dependent(doc, commit)

    def parse_accepted(self, doc):
        value = doc.fields["Accepted"]
        match = re.fullmatch(r"([0-9]{1,4})(?:; (OD-[0-9-]+(?:, OD-[0-9-]+)*))?", value)
        if not match:
            self.fail("R2", doc.rel, None, "Accepted must be 'N' or 'N; OD-..., OD-...'")
            return None, []
        count = int(match.group(1))
        ids = match.group(2).split(", ") if match.group(2) else []
        if len(ids) != count:
            self.fail("R2", doc.rel, None, "Accepted counts a different number of findings than it lists")
        if len(set(ids)) != len(ids):
            self.fail("R2", doc.rel, None, "Accepted lists an owner-decision id twice")
        return count, ids

    def parse_reports(self, doc):
        parts = doc.fields["Reports"].split("; ")
        entries = []
        for part in parts:
            match = re.fullmatch(r"(\S+) sha256:([0-9a-f]{64})", part)
            if not match:
                self.fail("R2", doc.rel, None, "Reports must be '<path> sha256:<64 hex>' entries separated by '; '")
                return None
            entries.append((match.group(1), match.group(2)))
        if len(entries) != 4 or len({p for p, _ in entries}) != 4:
            self.fail("R2", doc.rel, None, "Reports must list exactly four distinct report files")
            return None
        return entries

    def report_text(self, doc, path):
        if os.path.isabs(path) or ".." in path.split("/") or "\\" in path:
            self.fail("R2", doc.rel, None, "a report path must be relative and free of '..'")
            return None
        reviews = os.path.join(self.root, "docs", "designs", doc.stem + ".reviews")
        full = os.path.join(self.root, path)
        if os.path.realpath(full) != os.path.abspath(full) or os.path.islink(full):
            self.fail("R2", doc.rel, None, f"a symbolic link is involved in the report path {shown(path)}")
            return None
        real_reviews = os.path.realpath(reviews)
        if os.path.commonpath([real_reviews, os.path.realpath(full)]) != real_reviews:
            self.fail("R2", doc.rel, None, f"the report {shown(path)} is not under docs/designs/{doc.stem}.reviews/")
            return None
        if not os.path.isfile(full):
            self.fail("R2", doc.rel, None, f"the report {shown(path)} is not a regular file")
            return None
        code, out = self.git("--literal-pathspecs", "ls-files", "-s", "--", path)
        if code != 0 or not out.startswith("100644 "):
            self.fail("R2", doc.rel, None, f"the report {shown(path)} is not a tracked regular file (mode 100644)")
            return None
        with open(full, "rb") as handle:
            data = handle.read()
        return data

    def check_reports(self, doc, reports):
        reviewers, findings = [], 0
        for path, recorded in reports:
            data = self.report_text(doc, path)
            if data is None:
                continue
            if hashlib.sha256(data).hexdigest() != recorded:
                self.fail("R2", doc.rel, None, f"the SHA-256 of {shown(path)} differs from the recorded one")
                continue
            text = data.decode("utf-8", errors="replace")
            if LOCAL_PATH_RE.search(text):
                self.fail("R2", doc.rel, None, f"{shown(path)} holds an absolute path of a machine")
            found = self.parse_report(doc, path, text)
            if found is None:
                continue
            reviewers.append(found[0])
            findings += found[1]
        if sorted(reviewers) != sorted(REVIEWERS):
            self.fail("R2", doc.rel, None, "the four reports must be written by egm-review, architecture, security and correctness")
            return None
        return findings

    def parse_report(self, doc, path, text):
        lines = text.split("\n")
        flags = fence_flags(lines)
        seen = {key: [] for key in ("Doc", "Reviewed-commit", "Reviewer", "EGM-hash", "Agent-blob")}
        counts = []
        for line, fenced in zip(lines, flags):
            if fenced:
                continue
            for key in seen:
                if line.startswith(key + ": "):
                    seen[key].append(line[len(key) + 2:])
            if line.startswith("Counts:"):
                counts.append(line)
        for key in ("Doc", "Reviewed-commit", "Reviewer"):
            if len(seen[key]) != 1:
                self.fail("R2", doc.rel, None, f"{shown(path)} needs exactly one '{key}:' line")
                return None
        if len(counts) != 1 or not COUNTS_RE.fullmatch(counts[0]):
            self.fail("R2", doc.rel, None, f"{shown(path)} needs exactly one well formed Counts line")
            return None
        reviewer = seen["Reviewer"][0]
        if seen["Doc"][0] != doc.rel:
            self.fail("R2", doc.rel, None, f"{shown(path)} names another doc")
        if seen["Reviewed-commit"][0] != doc.fields["Reviewed-commit"]:
            self.fail("R2", doc.rel, None, f"{shown(path)} was written for another Reviewed-commit")
        if reviewer == "egm-review":
            if seen["EGM-hash"] != [doc.fields["EGM-hash"]]:
                self.fail("R2", doc.rel, None, f"the EGM-hash of {shown(path)} differs from the header")
        elif reviewer in REVIEWERS:
            if len(seen["Agent-blob"]) != 1 or not HEX40.fullmatch(seen["Agent-blob"][0]):
                self.fail("R2", doc.rel, None, f"{shown(path)} needs exactly one Agent-blob line of 40 hex characters")
            else:
                self.agent_blobs[(doc.rel, reviewer)] = seen["Agent-blob"][0]
        parsed = COUNTS_RE.fullmatch(counts[0])
        assert parsed is not None
        return reviewer, int(parsed.group(1)) + int(parsed.group(2))

    def check_acceptances(self, doc, ids, paths):
        for accepted in ids:
            row = self.citable(accepted)
            anchored = row is not None and any(
                re.search(re.escape(path) + r"#[0-9]+", row["Covers"]) for path in paths)
            if not anchored:
                self.fail("R2", doc.rel, None, f"{accepted} is not an owner row that covers '<report path>#<n>'")

    def check_commit_dependent(self, doc, commit):
        old = self.base_text(doc.rel)
        if old is not None and old == doc.text:
            return  # unchanged: the commit may be unreachable after a squash merge
        if self.git("rev-parse", "--verify", "--quiet", f"{commit}^{{commit}}")[0] != 0:
            if self.strict:
                self.fail("R2", doc.rel, None, "the Reviewed-commit is not a commit of this clone")
            else:
                self.warnings.append(f"{doc.rel}: the Reviewed-commit is not in this clone; R3 skipped")
            return
        for reviewer in REVIEWERS[1:]:
            recorded = self.agent_blobs.get((doc.rel, reviewer))
            code, out = self.git("rev-parse", "--verify", "--quiet", f"{commit}:.claude/agents/{reviewer}-reviewer.md")
            if recorded is not None and (code != 0 or out.strip() != recorded):
                self.fail("R2", doc.rel, None, f"the Agent-blob of the {reviewer} report differs from the agent at the Reviewed-commit")
        self.check_body(doc, commit)

    def check_body(self, doc, commit):
        code, text = self.git("show", f"{commit}:{doc.rel}")
        if code != 0:
            self.fail("R3", doc.rel, None, "the doc does not exist at the Reviewed-commit")
            return
        if "\r" in text or "\r" in doc.text:
            self.fail("R3", doc.rel, None, "a carriage return is not allowed in an approved doc")
            return
        old = Doc(doc.rel, text)
        if old.hend is None or doc.hend is None:
            self.fail("R3", doc.rel, None, "a header terminator is missing at the Reviewed-commit or now")
            return
        line = first_difference(header_core(old), header_core(doc), doc.hend + 1)
        if line:
            self.fail("R3", doc.rel, line, "the header changed outside the status block fields")
        old_ids = {task.id for task in old.tasks}
        line = first_difference(normalized_body(old, None), normalized_body(doc, old_ids), len(doc.lines))
        if line:
            self.fail("R3", doc.rel, line, "the body changed beyond ticks, evidence and follow-up tasks")
        new_by_id = {task.id: task for task in doc.tasks}
        for task in old.tasks:
            current = new_by_id.get(task.id)
            if task.marker is None or current is None:
                continue
            before = old.lines[task.marker:task.end + 1]
            after = doc.lines[current.marker:current.end + 1] if current.marker is not None else []
            if after[:len(before)] != before:
                self.fail("R3", doc.rel, current.start + 1, f"the existing evidence of {task.id} was changed or removed")

    # R4 across docs, R6 -----------------------------------------------------------------
    def check_cross_doc(self, docs):
        ids, prefixes = {}, {}
        for doc in docs:
            for task in doc.tasks:
                ids.setdefault(task.id, []).append(doc.rel)
            if doc.number is None and "Task-prefix" in doc.fields:
                prefixes.setdefault(doc.fields["Task-prefix"], []).append(doc.rel)
        for task_id, owners in sorted(ids.items()):
            if len(set(owners)) > 1:
                self.fail("R4", owners[1], None, f"{task_id} is also defined in {owners[0]}")
        for prefix, owners in sorted(prefixes.items()):
            if len(owners) > 1:
                self.fail("R4", owners[1], None, f"Task-prefix {prefix} is also declared by {owners[0]}")

    def check_numbers(self, docs):
        slugs = {}
        for doc in docs:
            if doc.number is not None:
                slugs.setdefault(doc.number, set()).add(doc.slug)
        for doc in docs:
            if doc.number is not None and len(slugs[doc.number]) > 1:
                self.fail("R6", doc.rel, None, f"the number {doc.number} is used with different slugs")
        specs = os.path.join(self.root, "specs")
        if os.path.isdir(specs):
            for name in sorted(os.listdir(specs)):
                match = re.fullmatch(r"([0-9]{3})-(.+)", name)
                if match and os.path.isdir(os.path.join(specs, name)) and match.group(1) in slugs:
                    if match.group(2) not in slugs[match.group(1)]:
                        self.fail("R6", f"specs/{name}", None, f"the number {match.group(1)} is used by a design doc with another slug")

    # R8 ---------------------------------------------------------------------------------
    def amendment_base(self):
        """The ref an amendment is compared with. A pull request into main (a release or a hotfix) carries amendments that
        were each checked when they entered develop, while main holds an older constitution; it is compared with the point
        where the branch left develop, so an amendment made on the branch itself still needs that text as its source."""
        if self.base != "origin/main":
            return self.base
        code, out = self.git("merge-base", "HEAD", "origin/develop")
        return out.strip() if code == 0 and out.strip() else self.base

    def base_constitution(self):
        """(blob id, text) of the constitution at the amendment base, wherever it lived, or (None, None)."""
        base = self.amendment_base()
        for rel in ("docs/constitution.md", ".specify/memory/constitution.md"):
            code, blob = self.git("rev-parse", "--verify", "--quiet", f"{base}:{rel}")
            if code == 0:
                shown_code, body = self.git("show", f"{base}:{rel}")
                return blob.strip(), body if shown_code == 0 else None
        return None, None

    def check_amendment(self, text, source, report_rel):
        """A changed constitution needs a higher version and a Previous-source that is the base text."""
        if not self.base_ok():
            return
        blob, base_text = self.base_constitution()
        if base_text is None or base_text == text:
            return
        if source != blob:
            self.fail("R8", report_rel, None, "Previous-source is not the constitution at the base ref")
        now = re.search(r"\*\*Version\*\*: ([0-9]{1,6})\.([0-9]{1,6})\.([0-9]{1,6})", text)
        before = re.search(r"\*\*Version\*\*: ([0-9]{1,6})\.([0-9]{1,6})\.([0-9]{1,6})", base_text)
        if now and before and tuple(map(int, now.groups())) <= tuple(map(int, before.groups())):
            self.fail("R8", "docs/constitution.md", None, "the constitution changed but its version did not increase")

    def check_constitution(self):
        text = self.read("docs/constitution.md")
        if text is None:
            return
        rel = "docs/constitution.md"
        match = re.search(r"\*\*Version\*\*: ([0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6})", text)
        versions = []
        for path in glob.glob(os.path.join(glob.escape(self.root), "docs", "governance", "constitution-*-sync-impact.md")):
            found = re.fullmatch(r"constitution-([0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6})-sync-impact\.md", os.path.basename(path))
            if found:
                versions.append(found.group(1))
        if not match or not versions:
            self.fail("R8", rel, None, "the version line or the Sync Impact Report is missing")
            return
        newest = max(versions, key=lambda v: tuple(int(p) for p in v.split(".")))
        if match.group(1) != newest:
            self.fail("R8", rel, None, f"the version {match.group(1)} differs from the newest Sync Impact Report {newest}")
            return
        report_rel = f"docs/governance/constitution-{newest}-sync-impact.md"
        report = self.read(report_rel) or ""
        source = re.search(r"(?m)^Previous-source: ([0-9a-f]{40})$", report)
        table = {}
        for line in report.split("\n"):
            cells = split_row(line) if line.startswith("|") else []
            if len(cells) == 4 and cells[0] in ROMAN:
                table[cells[0]] = cells
        if not source or set(table) != set(ROMAN):
            self.fail("R8", report_rel, None, "the report lacks 'Previous-source:' or a row for each principle I to VIII")
            return
        self.check_amendment(text, source.group(1), report_rel)
        code, previous = self.git("cat-file", "blob", source.group(1))
        if code != 0:
            self.fail("R8", report_rel, None, "Previous-source is not a blob of this clone")
            return
        current_blocks, previous_blocks = principle_blocks(text), principle_blocks(previous)
        for roman in ROMAN:
            _, disposition, was, now = table[roman]
            if current_blocks.get(roman) != now:
                self.fail("R8", report_rel, None, f"the hash of principle {roman} differs from docs/constitution.md")
            if previous_blocks.get(roman) != was:
                self.fail("R8", report_rel, None, f"the previous hash of principle {roman} differs from Previous-source")
            if disposition == "KEEP" and was != now:
                self.fail("R8", report_rel, None, f"principle {roman} is KEEP but its text changed")


def main(argv):
    parser = argparse.ArgumentParser(description="Check the design docs under docs/designs.")
    parser.add_argument("--root", default=os.getcwd(), help="repository root (default: the current directory)")
    args = parser.parse_args(argv[1:])
    probe = subprocess.run(["git", "-C", args.root, "rev-parse", "--git-dir"], capture_output=True)
    if probe.returncode != 0:
        print("check-design-docs: not inside a git repository", file=sys.stderr)
        return 2
    checker = Checker(args.root)
    checker.run()
    for warning in checker.warnings:
        print(f"check-design-docs: warning: {warning}", file=sys.stderr)
    if checker.failures:
        print("\n".join(checker.failures))
        return 1
    print("design docs ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
