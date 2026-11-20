"""Judges the rounds of design 041's spike T041-35, second run, by its pre-fixed rule (T041-35, "Second run"; gate (2b)
per OD-2026-11-20-5), from the spike-carry-json lines: python3 -I carry_spike_judge.py <OUT> [--count-valid]."""
import json, math, statistics, sys
OUT = sys.argv[1]
HALF = 10
rounds = {}
import glob, os
# every round present, in its number's order (a round number is the fixed order of D041-104 on the runners)
numbers = sorted(int(os.path.basename(p)[len("second-round-"):-len(".log")]) for p in glob.glob(f"{OUT}/second-round-*.log"))
for r in numbers:
    text = open(f"{OUT}/second-round-{r}.log").read()
    rows = [json.loads(l.split("spike-carry-json ", 1)[1]) for l in text.splitlines() if "spike-carry-json " in l]
    quiet = open(f"{OUT}/second-round-{r}.quiet").read().strip() == "1"
    rounds[r] = ({(x["label"], x["replicas"]): x for x in rows}, quiet)
res = {}
for r, (m, quiet) in rounds.items():
    out = {"quiet": quiet, "valid": True, "why": []}
    for size in (1, 8):
        g = lambda l: m.get((l, size))
        o1, o1b, oe, oeb, a, f, gg = (g(x) for x in ("off-1K", "off-1K again", "off-{}", "off-{} again", "a", "f", "g"))
        if not all([o1, o1b, oe, oeb, a, f, gg]):
            out["valid"] = False; out["why"].append(f"incomplete at {size}"); continue
        for x, y, n in ((o1, o1b, "off-1K"), (oe, oeb, "off-{}")):
            d = abs(y["per_second"] - x["per_second"]) / x["per_second"]
            out[f"drift {n} {size}"] = round(d, 4)
            if d > 0.10:
                out["valid"] = False; out["why"].append(f"{n} at {size} drifted {d:.1%}")
        for w in (a, f, gg):
            notw = w["carry_calls"] - w["carry_written"]
            if w["carry_calls"] and notw / w["carry_calls"] > 0.001:
                out["valid"] = False; out["why"].append(f"{w['label']} at {size}: {notw} carries not written")
        out[f"g1 {size}"] = a["per_second"] / o1["per_second"]
        out[f"g2 {size}"] = a["complete_p99_ms"] / o1["complete_p99_ms"] - 1
        allowance = max(0.10 * o1["complete_p99_ms"], 2.0)
        out[f"g2b allowance {size}"] = allowance
        out[f"g2b over {size}"] = a["complete_p99_ms"] - o1["complete_p99_ms"] - allowance
        out[f"g3 {size}"] = f["per_second"] / oe["per_second"]
        out[f"g4 {size}"] = math.ceil((gg["per_second"] / size) * gg["s_p99_ms"] / 1000 / 0.7)
        out[f"g4 refusals {size}"] = gg["ticket_refusals"] / max(gg["batches"], 1)
    for l in ("h", "d"):
        x = m.get((l, 1))
        if x and x["carry_calls"] and (x["carry_calls"] - x["carry_written"]) / x["carry_calls"] > 0.001:
            out["valid"] = False; out["why"].append(f"{l}: carries not written")
    res[r] = out
valid = [r for r in sorted(res) if res[r]["valid"]][:3]
if "--count-valid" in sys.argv:
    print(len(valid)); sys.exit(0)
print(json.dumps(res, indent=1))
print("valid rounds used:", valid)
if len(valid) < 3:
    print("NOT DECIDED"); sys.exit(0)
gates = {"g1": lambda v: v >= 0.9, "g2b over": lambda v: v <= 0.0, "g3": lambda v: v >= 0.9}
verdict = {}
for size in (1, 8):
    for gate, ok in gates.items():
        vals = [res[r][f"{gate} {size}"] for r in valid]
        quiet_meeting = sum(1 for r in valid if res[r]["quiet"] and ok(res[r][f"{gate} {size}"]))
        verdict[f"{gate} {size}"] = {"values": vals, "median": statistics.median(vals),
                                     "pass": ok(statistics.median(vals)) and quiet_meeting >= 2}
    verdict[f"g2 (+10%, recorded) {size}"] = {"values": [res[r][f"g2 {size}"] for r in valid]}
    verdict[f"g2b allowance ms {size}"] = {"values": [res[r][f"g2b allowance {size}"] for r in valid]}
    vals = [(res[r][f"g4 {size}"], res[r][f"g4 refusals {size}"]) for r in valid]
    verdict[f"g4 {size}"] = {"values": vals, "pass": all(t <= HALF and rf < 0.01 for t, rf in vals)}
print(json.dumps(verdict, indent=1))
print("PASS" if all(v["pass"] for v in verdict.values() if "pass" in v) else "FAIL")
