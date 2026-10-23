"""Topographic contour lines for direction A (PDA map). Code-native texture: smooth noise -> marching squares -> SVG.

Output: design/shared/img/zone_topo.svg (colours in the inline <style>; index contours = class "ix").
"""
from pathlib import Path

import numpy as np
from scipy.ndimage import gaussian_filter
from skimage.measure import find_contours

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "design" / "shared" / "img" / "zone_topo.svg"
W, H = 800, 640  # svg units (the map panel scales it with preserveAspectRatio slice)


def field(seed: int = 23) -> np.ndarray:
    rng = np.random.default_rng(seed)
    gh, gw = 160, 200
    z = np.zeros((gh, gw))
    for sigma, amp in [(26, 1.0), (12, 0.45), (6, 0.18), (3, 0.06)]:
        z += amp * gaussian_filter(rng.standard_normal((gh, gw)), sigma, mode="wrap")
    # a valley running SW-NE (river bed) keeps it from looking like random blobs
    yy, xx = np.mgrid[0:gh, 0:gw]
    d = (xx * 0.62 - yy - 10) / 70.0
    z -= 0.9 * np.exp(-d * d * 3.0) * z.std()
    return (z - z.min()) / (z.max() - z.min())


def main() -> None:
    z = field()
    gh, gw = z.shape
    sx, sy = W / (gw - 1), H / (gh - 1)
    levels = np.linspace(0.04, 0.96, 31)
    parts = []
    for i, lv in enumerate(levels):
        cls = ' class="ix"' if i % 5 == 0 else ""
        for c in find_contours(z, lv):
            if len(c) < 8:
                continue
            c = c[:: 2] if len(c) > 40 else c
            pts = " ".join(f"{p[1] * sx:.1f},{p[0] * sy:.1f}" for p in c)
            parts.append(f'<polyline{cls} points="{pts}"/>')
    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" preserveAspectRatio="xMidYMid slice" '
        f'fill="none" stroke-linejoin="round">'
        + "<style>polyline{stroke:#323C2B;stroke-width:1}.ix{stroke:#4A5740;stroke-width:1.5}</style>"
        + "".join(parts)
        + "</svg>"
    )
    OUT.write_bytes(svg.encode())
    print(OUT.name, len(parts), "lines", OUT.stat().st_size // 1024, "KB")


if __name__ == "__main__":
    main()
