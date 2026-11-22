#!/usr/bin/env python3
"""Fit how Maschine turns MK3 pad pressure into velocity, from mk3-velocity-measure's CSVs.

    python -I crates/mk3/examples/mk3-velocity-fit.py <folder or .csv ...>

For each recording (one Pad Sensitivity / Velocity Scaling setting) it pairs each note
Maschine sent with the press that caused it, then:

1. tries several readings of a press (first report, peak of the first k reports, peak
   within w ms, peak before the note left) and ranks them by how well a monotonic
   function of each predicts the velocity: the reading Maschine uses fits almost exactly;
2. for the best reading, prints each setting's points, threshold and a fitted curve
   v = 127 * clip((p - p0) / (p1 - p0), 0, 1) ** g;
3. writes report.html (scatter plot and tables) next to the data.

Standard library only.
"""

import csv
import html
import math
import os
import sys
from pathlib import Path

MATCH_MS = 100.0


def load(path):
    label = Path(path).stem
    hid, notes = [], []
    with open(path, newline="") as f:
        first = f.readline()
        if first.startswith("# label="):
            label = first[len("# label="):].split(" midi=")[0].strip()
        else:
            f.seek(0)
        for row in csv.DictReader(f):
            t = float(row["t_ms"])
            if row["source"] == "hid":
                hid.append((t, int(row["pad"]), int(row["event"]), int(row["pressure"])))
            elif row["source"] == "midi" and row["status"] and row["velocity"]:
                status, vel = int(row["status"]), int(row["velocity"])
                if status & 0xF0 == 0x90 and vel > 0:
                    notes.append((t, vel))
    return label, presses_from(hid), notes


def presses_from(hid):
    held, done = {}, []
    for t, pad, event, p in hid:
        if event in (0, 1):
            if pad in held:
                done.append(held.pop(pad))
            held[pad] = {"pad": pad, "t": t, "event": event, "reports": [(t, p)], "end": None}
        elif event == 4 and pad in held:
            held[pad]["reports"].append((t, p))
        elif event in (2, 3) and pad in held:
            pr = held.pop(pad)
            pr["end"] = t
            done.append(pr)
    done.extend(held.values())
    done.sort(key=lambda pr: pr["t"])
    return done


def match(presses, notes):
    pairs, used = [], set()
    for tn, vel in notes:
        best = None
        for i, pr in enumerate(presses):
            if i in used or pr["t"] > tn or tn - pr["t"] >= MATCH_MS:
                continue
            if best is None or pr["t"] > presses[best]["t"]:
                best = i
        if best is not None:
            used.add(best)
            pairs.append((presses[best], tn, vel))
    unmatched = [pr for i, pr in enumerate(presses) if i not in used]
    return pairs, unmatched


def peak_first(k):
    return lambda pr, tn: max(p for _, p in pr["reports"][:k])


def peak_within(w):
    return lambda pr, tn: max(p for t, p in pr["reports"] if t - pr["t"] <= w)


def peak_before_note(pr, tn):
    return max(p for t, p in pr["reports"] if t <= tn)


FEATURES = {"first": peak_first(1)}
FEATURES.update({f"peak of first {k}": peak_first(k) for k in range(2, 9)})
FEATURES.update({f"peak within {w} ms": peak_within(w) for w in (1, 2, 3, 4, 5, 6, 8, 10, 12, 16, 24)})
FEATURES["peak before note"] = peak_before_note


def isotonic(xs, ys):
    """Least-squares non-decreasing fit of ys over xs (ties pooled). Returns fitted ys
    in the input order."""
    order = sorted(range(len(xs)), key=lambda i: xs[i])
    blocks = []  # [sum, count, indices]
    for i in order:
        blocks.append([ys[i], 1, [i]])
        while len(blocks) > 1 and blocks[-2][0] / blocks[-2][1] >= blocks[-1][0] / blocks[-1][1]:
            s, c, idx = blocks.pop()
            blocks[-1][0] += s
            blocks[-1][1] += c
            blocks[-1][2] += idx
    fit = [0.0] * len(xs)
    for s, c, idx in blocks:
        for i in idx:
            fit[i] = s / c
    return fit


def monotone_error(xs, ys):
    fit = isotonic(xs, ys)
    errs = [y - f for y, f in zip(ys, fit)]
    rmse = math.sqrt(sum(e * e for e in errs) / len(errs))
    return rmse, max(abs(e) for e in errs)


def curve(p, p0, p1, g):
    u = min(1.0, max(0.0, (p - p0) / (p1 - p0)))
    return 127.0 * u ** g


def fit_curve(xs, ys):
    best = None
    gs = [math.exp(math.log(0.2) + i * (math.log(5.0) - math.log(0.2)) / 60) for i in range(61)]
    for p0 in range(0, 1601, 25):
        for p1 in range(max(p0 + 100, 400), 4096, 50):
            for g in gs:
                sse = sum((max(1.0, curve(x, p0, p1, g)) - y) ** 2 for x, y in zip(xs, ys))
                if best is None or sse < best[0]:
                    best = (sse, p0, p1, g)
    sse, p0, p1, g = best
    return p0, p1, g, math.sqrt(sse / len(xs))


def main(argv):
    paths = []
    for a in argv:
        p = Path(a)
        paths += sorted(p.glob("*.csv")) if p.is_dir() else [p]
    if not paths:
        print(__doc__)
        return 2
    runs = []
    for path in paths:
        label, presses, notes = load(path)
        pairs, unmatched = match(presses, notes)
        lat = sorted(tn - pr["t"] for pr, tn, _ in pairs)
        print(f"{label}: {len(presses)} presses, {len(notes)} notes, {len(pairs)} paired, "
              f"{len(unmatched)} presses without a note"
              + (f"; note {lat[0]:.1f}..{lat[len(lat) // 2]:.1f}..{lat[-1]:.1f} ms after the press" if lat else ""))
        if len(pairs) >= 5:
            runs.append((label, path, pairs, unmatched))
    if not runs:
        print("no recording has 5 or more paired notes")
        return 1

    print("\nHow well each reading of a press predicts the velocity (monotonic fit, velocity steps):")
    scores = []
    for name, fn in FEATURES.items():
        total, worst, n = 0.0, 0.0, 0
        for _, _, pairs, _ in runs:
            xs = [fn(pr, tn) for pr, tn, _ in pairs]
            ys = [v for _, _, v in pairs]
            rmse, mx = monotone_error(xs, ys)
            total += rmse * rmse * len(xs)
            n += len(xs)
            worst = max(worst, mx)
        scores.append((math.sqrt(total / n), worst, name))
    scores.sort()
    for rmse, worst, name in scores:
        print(f"  {name:22s} rmse {rmse:6.2f}  worst {worst:6.1f}")
    best_name = scores[0][2]
    fn = FEATURES[best_name]
    print(f"\nBest: {best_name}\n")

    report = []
    for label, path, pairs, unmatched in runs:
        pts = sorted((fn(pr, tn), v) for pr, tn, v in pairs)
        xs, ys = [x for x, _ in pts], [y for _, y in pts]
        silent = [max(p for _, p in pr["reports"]) for pr in unmatched]
        p0, p1, g, rmse = fit_curve(xs, ys)
        print(f"{label}: lightest note {xs[0]} -> {ys[0]}, hardest {xs[-1]} -> {ys[-1]}; "
              f"loudest press with no note peaked at {max(silent) if silent else '-'}")
        print(f"  fit: v = 127 * clip((p - {p0}) / ({p1} - {p0}), 0, 1) ^ {g:.3f}   rmse {rmse:.2f}")
        print("  points (pressure -> velocity): " + ", ".join(f"{x}->{y}" for x, y in pts))
        report.append((label, pts, silent, (p0, p1, g, rmse)))

    out = Path(runs[0][1]).parent / "report.html"
    write_html(out, best_name, scores, report)
    print(f"\nreport: {out.resolve()}")
    return 0


COLORS = ["#2a6fdb", "#d9480f", "#2b8a3e", "#ae3ec9", "#e8590c", "#1098ad", "#c2255c", "#5f3dc4",
          "#74b816", "#f08c00", "#495057", "#0b7285"]


def write_html(path, best_name, scores, report):
    W, H, L, B = 760, 440, 50, 36
    sx = lambda p: L + (W - L - 10) * p / 4095
    sy = lambda v: H - B - (H - B - 10) * v / 127
    svg = [f'<svg viewBox="0 0 {W} {H}" width="100%" role="img" aria-label="velocity against pressure">']
    for v in (0, 32, 64, 96, 127):
        svg.append(f'<line x1="{L}" x2="{W - 10}" y1="{sy(v):.1f}" y2="{sy(v):.1f}" class="grid"/>'
                   f'<text x="{L - 6}" y="{sy(v) + 4:.1f}" text-anchor="end">{v}</text>')
    for p in (0, 1024, 2048, 3072, 4095):
        svg.append(f'<text x="{sx(p):.1f}" y="{H - B + 16}" text-anchor="middle">{p}</text>')
    svg.append(f'<text x="{(L + W) / 2}" y="{H - 4}" text-anchor="middle">pressure ({html.escape(best_name)})</text>')
    rows = []
    for i, (label, pts, silent, (p0, p1, g, rmse)) in enumerate(report):
        c = COLORS[i % len(COLORS)]
        path_d = " ".join(f"{'M' if k == 0 else 'L'}{sx(p):.1f},{sy(max(1.0, curve(p, p0, p1, g))):.1f}"
                          for k, p in enumerate(range(0, 4096, 32)))
        svg.append(f'<path d="{path_d}" fill="none" stroke="{c}" stroke-width="1.5" opacity="0.6"/>')
        svg += [f'<circle cx="{sx(x):.1f}" cy="{sy(y):.1f}" r="2.6" fill="{c}"/>' for x, y in pts]
        svg += [f'<circle cx="{sx(x):.1f}" cy="{sy(0):.1f}" r="2.6" fill="none" stroke="{c}"/>' for x in silent]
        rows.append(f'<tr><td><span class="sw" style="background:{c}"></span>{html.escape(label)}</td>'
                    f'<td>{len(pts)}</td><td>{pts[0][0]} &rarr; {pts[0][1]}</td>'
                    f'<td>{max(silent) if silent else "&ndash;"}</td>'
                    f'<td>{p0}</td><td>{p1}</td><td>{g:.3f}</td><td>{rmse:.2f}</td></tr>')
    svg.append("</svg>")
    score_rows = "".join(f"<tr><td>{html.escape(n)}</td><td>{r:.2f}</td><td>{w:.1f}</td></tr>" for r, w, n in scores)
    doc = f"""<!doctype html><html><head><meta charset="utf-8"><title>Pad velocity fit</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>
:root {{ --bg:#fff; --fg:#1c1c1c; --muted:#666; --line:#ddd; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg:#16181b; --fg:#e8e8e8; --muted:#999; --line:#333; }} }}
body {{ background:var(--bg); color:var(--fg); font:14px/1.45 system-ui, sans-serif; margin:0 auto; max-width:820px; padding:16px; }}
svg text {{ fill:var(--muted); font-size:11px; }} .grid {{ stroke:var(--line); }}
table {{ border-collapse:collapse; margin:12px 0; }} td, th {{ border-bottom:1px solid var(--line); padding:3px 10px; text-align:left; }}
.sw {{ display:inline-block; width:10px; height:10px; border-radius:2px; margin-right:6px; }}
p {{ color:var(--muted); }}
</style></head><body>
<h1>Pad velocity fit</h1>
<p>Filled dots: notes Maschine sent. Rings on the axis: presses that sent no note. Lines: v = 127 &middot; clip((p &minus; p0)/(p1 &minus; p0))<sup>g</sup>.</p>
{''.join(svg)}
<table><tr><th>setting</th><th>notes</th><th>lightest note</th><th>loudest silent press</th><th>p0</th><th>p1</th><th>g</th><th>rmse</th></tr>{''.join(rows)}</table>
<h2>Which reading of a press</h2>
<table><tr><th>reading</th><th>rmse</th><th>worst</th></tr>{score_rows}</table>
</body></html>"""
    path.write_text(doc, encoding="utf-8")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
