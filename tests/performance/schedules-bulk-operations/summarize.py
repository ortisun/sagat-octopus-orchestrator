#!/usr/bin/env python3
"""Reads the fragments a run of run.sh left (<out>/fragments/*.jsonl, <out>/status.tsv, <out>/load.log) and prints the tables of the
report: for every scenario the median of the repetitions with the smallest and the largest in brackets, N, and every bound with
how many repetitions held it.

    python3 -I tests/performance/schedules-bulk-operations/summarize.py <out>
"""
import glob
import json
import os
import re
import statistics
import sys


def fragments(out):
    found = {}
    for path in sorted(glob.glob(os.path.join(out, "fragments", "*.jsonl"))):
        with open(path, encoding="utf-8") as handle:
            for line in handle:
                line = line.strip()
                if line:
                    record = json.loads(line)
                    found.setdefault(record["scenario"], []).append(record)
    return found


def dig(record, path):
    value = record
    for key in path.split("."):
        if isinstance(value, list) and key.isdigit() and int(key) < len(value):
            value = value[int(key)]
        elif isinstance(value, dict) and key in value:
            value = value[key]
        else:
            return None
    return value


def numbers(records, path):
    values = [dig(record, path) for record in records]
    return [float(v) for v in values if isinstance(v, (int, float)) and not isinstance(v, bool)]


def fmt(value, digits=2):
    if value is None:
        return "n/a"
    if abs(value) >= 1000:
        return f"{value:,.0f}"
    if abs(value) >= 100:
        return f"{value:.0f}"
    if abs(value) >= 10:
        return f"{value:.1f}"
    return f"{value:.{digits}f}"


def spread(records, path, digits=2):
    """median [min - max] of a numeric path over the repetitions."""
    values = numbers(records, path)
    if not values:
        return "n/a"
    if len(values) == 1:
        return fmt(values[0], digits)
    return f"{fmt(statistics.median(values), digits)} [{fmt(min(values), digits)} - {fmt(max(values), digits)}]"


def table(header, rows):
    lines = ["| " + " | ".join(header) + " |", "|" + "|".join(["---"] * len(header)) + "|"]
    lines += ["| " + " | ".join(str(cell) for cell in row) + " |" for row in rows]
    return "\n".join(lines)


def load_average(text):
    match = re.search(r"load averages?:\s*([\d.]+)[ ,]+([\d.]+)[ ,]+([\d.]+)", text or "")
    return tuple(float(x) for x in match.groups()) if match else None


def loads(records):
    before = [load_average(dig(r, "load_before.uptime")) for r in records]
    after = [load_average(dig(r, "load_after.uptime")) for r in records]
    before = [b[0] for b in before if b]
    after = [a[0] for a in after if a]
    if not before or not after:
        return "n/a"
    return f"{min(before):.1f}-{max(before):.1f} / {min(after):.1f}-{max(after):.1f}"


def group(found, prefix):
    return {name: records for name, records in sorted(found.items()) if name.startswith(prefix)}


def by_size(name):
    return int(name.rsplit("-", 1)[1]) if name.rsplit("-", 1)[-1].isdigit() else 0


def section_firing(found):
    out = ["### Firing lag", ""]
    names = sorted(group(found, "firing-"), key=lambda n: (n.rsplit("-", 1)[0], by_size(n)))
    rows = []
    for name in names:
        r = found[name]
        rows.append([
            name, len(r),
            spread(r, "numbers.lag_s_product_histogram.p50"), spread(r, "numbers.lag_s_product_histogram.p95"),
            spread(r, "numbers.lag_s_product_histogram.max"),
            spread(r, "numbers.lag_s_database.p95"),
            spread(r, "numbers.outside_s_after_the_first_nominal_time_until_this_share_had_a_visible_run.95%"),
            spread(r, "numbers.outside_s_after_the_first_nominal_time_until_this_share_had_a_visible_run.100%"),
        ])
    out.append(table(["scenario", "N", "lag p50 s (product)", "p95 s (product)", "max s (product)", "p95 s (database)",
                      "95% visible s (outside)", "100% visible s (outside)"], rows))
    out += ["", "Lag is the time from the nominal time to the decision (`schedule_action_lag_seconds`, recorded in the engine process); "
            "`database` is `decided_at` less the nominal time; `outside` is when a separate connection polling every 100 ms first saw a "
            "run for that share of the schedules, counted from the first nominal time.", ""]
    rows = []
    for name in names:
        r = found[name]
        rows.append([
            name, spread(r, "numbers.actions_per_second_burst", 1), spread(r, "numbers.wall_s", 1),
            spread(r, "numbers.created_in_s", 1),
            spread(r, "numbers.gauge_max_s", 1), spread(r, "numbers.gauge_seconds_above_60", 1),
            spread(r, "numbers.gauge_longest_stretch_above_60", 1),
            f"{sum(1 for x in r if dig(x, 'numbers.alert_would_fire'))} of {len(r)}",
            spread(r, "numbers.rounds.0.rounds_that_worked", 0), spread(r, "numbers.rounds.0.work_round_ms.p50", 0),
            spread(r, "numbers.rounds.0.work_round_ms.max", 0),
        ])
    out.append(table(["scenario", "actions/s (burst)", "wall s", "schedules created in s", "gauge max s", "s above 60",
                      "longest stretch above 60 s", "SagatSchedulerLate fires", "rounds that worked (replica 1)",
                      "work round p50 ms", "work round max ms"], rows))
    out.append("")
    return out


def section_batch(found):
    out = ["### Batch rate", ""]
    ones = sorted(group(found, "batch-one-job-"), key=by_size)
    rows = []
    for name in ones:
        r = found[name]
        rows.append([
            name, len(r), spread(r, "numbers.rate_asked_per_s", 0), spread(r, "numbers.achieved_items_per_s", 1),
            spread(r, "numbers.selection_s_until_the_state_left_selecting"), spread(r, "numbers.wall_s", 1),
            spread(r, "numbers.gap_between_items_ms.p50", 1), spread(r, "numbers.gap_between_items_ms.p95", 1),
            spread(r, "numbers.gap_between_items_ms.max", 0), spread(r, "numbers.item_duration_s_product_histogram.p50", 4),
            spread(r, "numbers.item_duration_s_product_histogram.p95", 4),
        ])
    out.append(table(["scenario", "N", "asked items/s", "achieved items/s", "selection s", "wall s", "gap p50 ms", "gap p95 ms",
                      "gap max ms", "item duration p50 s", "p95 s"], rows))
    out.append("")
    rows = []
    for name in sorted(group(found, "batch-selection-and-drain-"), key=by_size):
        r = found[name]
        rows.append([name, len(r), spread(r, "numbers.selection_s"), spread(r, "numbers.selection_runs_per_s", 0),
                     spread(r, "numbers.drain_s"), spread(r, "numbers.drain_items_per_s", 0),
                     spread(r, "numbers.runs_started_in_s", 1)])
    out.append(table(["scenario", "N", "selection s", "runs selected/s", "drain s", "items drained/s", "runs started in s (setup)"], rows))
    out.append("")
    rows = []
    for name in sorted(list(group(found, "batch-sixteen-")) + list(group(found, "batch-one-big-")), key=lambda n: n):
        r = found[name]
        rows.append([name, len(r), spread(r, "numbers.demand_items_per_s", 0), spread(r, "numbers.aggregate_items_per_s", 1),
                     spread(r, "numbers.per_job_items_per_s.min", 1), spread(r, "numbers.per_job_items_per_s.p50", 1),
                     spread(r, "numbers.per_job_items_per_s.max", 1), spread(r, "numbers.first_item_s_per_job.max", 1),
                     spread(r, "numbers.last_item_s_per_job.min", 1), spread(r, "numbers.last_item_s_per_job.max", 1),
                     spread(r, "numbers.last_over_first_to_end", 2), spread(r, "numbers.wall_s", 1)])
    out.append(table(["scenario", "N", "demand items/s", "aggregate items/s", "slowest job items/s", "median job", "fastest job",
                      "latest first item s", "first job ends s", "last job ends s", "last/first end", "wall s"], rows))
    out.append("")
    return out


def section_claim(found):
    out = ["### Business-id claim, 32-way", ""]
    for name in sorted(group(found, "claim-")):
        r = found[name]
        rows = []
        for label, path in [
            ("one identifier, policy refuses: the start that wins", "numbers.hot_identifier_fail_ms.the_run_that_wins"),
            ("one identifier, policy refuses: the 31 refused", "numbers.hot_identifier_fail_ms.the_refused"),
            ("one identifier, USE_EXISTING: the run created", "numbers.hot_identifier_use_existing_ms.the_run_that_is_created"),
            ("one identifier, USE_EXISTING: the 31 answered with it", "numbers.hot_identifier_use_existing_ms.the_answers_with_the_existing_run"),
            ("32 identifiers, no contention", "numbers.thirty_two_identifiers_ms"),
            ("32 plain starts", "numbers.thirty_two_plain_starts_ms"),
        ]:
            rows.append([label, spread(r, path + ".n", 0), spread(r, path + ".p50", 1), spread(r, path + ".p95", 1),
                         spread(r, path + ".p99", 1), spread(r, path + ".max", 0)])
        out.append(f"{name} (N = {len(r)} repetitions; latencies in ms, median of the repetitions' medians [min - max])")
        out.append("")
        out.append(table(["shape", "samples per repetition", "p50 ms", "p95 ms", "p99 ms", "max ms"], rows))
        out += ["", f"Claim over plain start: p50 x{spread(r, 'numbers.claim_over_plain_start.p50')}, p95 x{spread(r, 'numbers.claim_over_plain_start.p95')}, "
                f"p99 x{spread(r, 'numbers.claim_over_plain_start.p99')}; a refused start over a plain one (p50) x{spread(r, 'numbers.refused_over_plain_start_p50')}.", ""]
    return out


def section_reset(found):
    out = ["### Reset", ""]
    records = found.get("reset", [])
    if not records:
        return out + ["(not run)", ""]
    cases = {}
    for record in records:
        for case in dig(record, "numbers.cases") or []:
            cases.setdefault(int(round(case["journal_events"], -2)) if case["journal_events"] > 300 else case["journal_events"], []).append(case)
    rows = []
    for key in sorted(cases):
        c = cases[key]
        rows.append([key, len(c), spread(c, "journal_events", 0), spread(c, "built_in_s", 0), spread(c, "points_ms", 0),
                     spread(c, "largest_accepted_point", 0), spread(c, "preview_ms.p50", 0), spread(c, "preview_ms.max", 0),
                     spread(c, "reset_ms.p50", 0), spread(c, "reset_ms.max", 0), spread(c, "source_row_locked_ms.max", 0),
                     spread(c, "rows_copied.p50", 0), spread(c, "verify_run_ms.p50", 0)])
    out.append(table(["journal (events, rounded)", "N", "events", "built in s", "points ms", "largest accepted point", "preview p50 ms",
                      "preview max ms", "reset p50 ms", "reset max ms", "source row locked max ms", "rows copied", "verify_run p50 ms"], rows))
    out.append("")
    rows = []
    for key in sorted(cases):
        c = cases[key]
        for label in ("first_past_the_ceiling", "near_20000"):
            if any(f"refused_{label}" in x for x in c):
                sub = [x[f"refused_{label}"] for x in c if f"refused_{label}" in x]
                rows.append([key, label.replace("_", " "), spread(sub, "sequence", 0), "RESET_PREFIX_TOO_LONG", len(sub),
                             spread(sub, "preview_ms.p50", 0), spread(sub, "preview_ms.max", 0),
                             spread(sub, "reset_ms.p50", 0), spread(sub, "reset_ms.max", 0)])
    if rows:
        out.append(table(["journal (events, rounded)", "request", "sequence", "reason", "N", "preview p50 ms", "preview max ms",
                          "reset p50 ms", "reset max ms"], rows))
        out.append("")
    return out


def section_triggers(found):
    out = ["### Trigger handover costs", ""]
    rows = []
    for name in sorted(group(found, "triggers-steady-"), key=by_size):
        r = found[name]
        rows.append([name, len(r), spread(r, "numbers.seeded_in_s", 0), spread(r, "numbers.unconverged", 0),
                     spread(r, "numbers.convergence_ms.p50", 0), spread(r, "numbers.convergence_ms.p95", 0),
                     spread(r, "numbers.convergence_ms.max", 0), spread(r, "numbers.unconverged_exists_ms.p50", 1),
                     spread(r, "numbers.drift_view_count_ms.p50", 0), spread(r, "numbers.sweep.windows", 0),
                     spread(r, "numbers.sweep.window_ms.p50", 0), spread(r, "numbers.sweep.window_ms.p95", 0),
                     spread(r, "numbers.sweep.window_ms.max", 0), spread(r, "numbers.sweep.total_s", 1)])
    out.append(table(["scenario", "N", "seeded in s", "unconverged", "count p50 ms", "count p95 ms", "count max ms",
                      "EXISTS check p50 ms", "drift view count p50 ms", "windows", "window p50 ms", "window p95 ms",
                      "window max ms", "sweep s"], rows))
    out.append("")
    out += ["The same fleets once every trigger has a schedule (the state the handover needs; the check under its lock must then look "
            "at every trigger and find none):", ""]
    rows = []
    for name in sorted(group(found, "triggers-steady-"), key=by_size):
        r = found[name]
        rows.append([name, len(r), spread(r, "numbers.converged.handover_check_exists_ms.p50", 0),
                     spread(r, "numbers.converged.handover_check_exists_ms.max", 0),
                     spread(r, "numbers.converged.convergence_ms.p50", 0), spread(r, "numbers.converged.sweep.windows", 0),
                     spread(r, "numbers.converged.sweep.window_ms.p50", 0), spread(r, "numbers.converged.sweep.window_ms.max", 0),
                     spread(r, "numbers.converged.sweep.total_s", 1)])
    out.append(table(["scenario", "N", "handover check (EXISTS) p50 ms", "max ms", "count p50 ms", "windows", "window p50 ms",
                      "window max ms", "sweep s"], rows))
    out.append("")
    rows = []
    for name in sorted(group(found, "triggers-backlog-"), key=by_size):
        r = found[name]
        rows.append([name, len(r), spread(r, "numbers.seeded_in_s", 0), spread(r, "numbers.unconverged", 0),
                     spread(r, "numbers.convergence_ms.p50", 0), spread(r, "numbers.convergence_ms.max", 0),
                     spread(r, "numbers.first_window_ms.p50", 0), spread(r, "numbers.first_window_ms.max", 0)])
    out.append(table(["scenario", "N", "seeded in s", "unconverged", "count p50 ms", "count max ms", "first window p50 ms",
                      "first window max ms"], rows))
    out.append("")
    rows = []
    for name in sorted(group(found, "triggers-backlog-"), key=by_size):
        r = found[name]
        for index in (0, 1):
            sub = [dig(x, f"numbers.variants.{index}") for x in r]
            sub = [s for s in sub if s]
            if not sub:
                continue
            rows.append([name, sub[0]["settings"], len(sub), spread(sub, "budget_s", 0), spread(sub, "schedules_made", 0),
                         spread(sub, "writes_per_second", 1), spread(sub, "rounds.rounds_that_worked", 0),
                         spread(sub, "rounds.work_round_ms.p50", 0), spread(sub, "rounds.work_round_ms.max", 0),
                         spread(sub, "drain_of_the_backlog_at_this_rate_minutes", 0)])
    out.append(table(["scenario", "settings", "N", "window s", "schedules made", "writes/s", "rounds that wrote",
                      "round p50 ms", "round max ms", "backlog drains in (minutes, extrapolated)"], rows))
    out.append("")
    return out


def section_verdicts(found):
    out = ["### Bounds asserted and whether they held", ""]
    rows = []
    for name in sorted(found):
        records = found[name]
        names = {}
        for record in records:
            for verdict in record["verdicts"]:
                names.setdefault(verdict["name"], []).append(verdict)
        for verdict_name, verdicts in sorted(names.items()):
            held = sum(1 for v in verdicts if v["held"])
            observed = "; ".join(v["observed"] for v in verdicts)
            rows.append([name, verdict_name, verdicts[0]["bound"], f"{held} of {len(verdicts)}", observed])
    out.append(table(["scenario", "bound", "asserted", "held in", "observed (each repetition)"], rows))
    out.append("")
    return out


def section_load(found, out_dir):
    out = ["### Load of the machine (1-minute load average, before / after each repetition: lowest-highest)", ""]
    rows = [[name, len(records), loads(records)] for name, records in sorted(found.items())]
    out.append(table(["scenario", "N", "before / after"], rows))
    out.append("")
    status = os.path.join(out_dir, "status.tsv")
    if os.path.exists(status):
        out += ["### Repetitions and their exit status (0 = every bound held)", ""]
        rows = []
        with open(status, encoding="utf-8") as handle:
            for line in handle:
                label, rep, code, seconds = line.rstrip("\n").split("\t")
                rows.append([label, rep, code, seconds])
        out.append(table(["scenario", "repetition", "exit status", "seconds"], rows))
        out.append("")
    return out


def main():
    out_dir = sys.argv[1]
    found = fragments(out_dir)
    lines = []
    for section in (section_firing, section_batch, section_claim, section_reset, section_triggers, section_verdicts):
        lines += section(found)
    lines += section_load(found, out_dir)
    print("\n".join(lines))


if __name__ == "__main__":
    main()
