"""Draws a small chart to examples/chart.png: what pixlens shows under the
Bash call that ran it. Needs matplotlib."""

import math
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

days = list(range(1, 31))
visits = [120 + 40 * math.sin(d / 4) + d * 3 for d in days]
signups = [v * 0.18 + 8 * math.cos(d / 3) for d, v in zip(days, visits)]

fig, ax = plt.subplots(figsize=(8, 4.5), dpi=120)
fig.patch.set_facecolor("#1e1f29")
ax.set_facecolor("#1e1f29")
ax.plot(days, visits, color="#ff7a59", linewidth=2.5, label="visits")
ax.fill_between(days, visits, color="#ff7a59", alpha=0.15)
ax.plot(days, signups, color="#5cc8ff", linewidth=2.5, label="sign-ups")
ax.set_title("September", color="white", fontsize=16, loc="left")
ax.tick_params(colors="#c8c8d0")
for side in ax.spines.values():
    side.set_color("#44475a")
ax.legend(facecolor="#1e1f29", edgecolor="#44475a", labelcolor="white")
out = Path(__file__).with_name("chart.png")
fig.savefig(out, bbox_inches="tight")
print(f"saved {out.name}")
