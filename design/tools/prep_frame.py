"""Clean the owner's GAMMA frame for the launcher mockups.

Source: anomaly-coop/docs/hud/hud_v2_mockup.png and hud_v21_mockup.png (same 1265x693 frame, different HUD overlays).
Takes the v2.1 frame, puts back the v2 pixels where v2.1 has name tags / caption, crops the compass strip off the top
and inpaints the in-game HUD in the bottom-left corner. Output: design/shared/img/zone_bunker.jpg (+ a wide crop).
"""
from pathlib import Path

import cv2
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
HUD = ROOT.parent / "anomaly-coop" / "docs" / "hud"
OUT = ROOT / "design" / "shared" / "img"


def main() -> None:
    v2 = np.array(Image.open(HUD / "hud_v2_mockup.png").convert("RGB"))
    v21 = np.array(Image.open(HUD / "hud_v21_mockup.png").convert("RGB"))[:693, :1265]
    frame = v21.copy()
    # v2.1 overlays that v2 does not have: two name tags, the caption bottom-right, the dark block top-right
    for x0, y0, x1, y1 in [(78, 192, 140, 248), (975, 305, 1036, 360), (810, 660, 1265, 693), (1054, 0, 1265, 50)]:
        frame[y0:y1, x0:x1] = v2[y0:y1, x0:x1]
    top = 66  # compass strip in both frames
    frame = frame[top:]
    # in-game HUD bottom-left (same in both): inpaint
    mask = np.zeros(frame.shape[:2], np.uint8)
    mask[560 - top:693 - top, 0:132] = 255
    mask[388 - top:402 - top, 150:165] = 255  # stray dot
    mask[124 - top:184 - top, 1176:1265] = 255  # in-game indicator top-right
    bgr = cv2.cvtColor(frame, cv2.COLOR_RGB2BGR)
    bgr = cv2.inpaint(bgr, mask, 9, cv2.INPAINT_TELEA)
    img = Image.fromarray(cv2.cvtColor(bgr, cv2.COLOR_BGR2RGB))
    OUT.mkdir(parents=True, exist_ok=True)
    img.save(OUT / "zone_bunker.jpg", quality=92)
    print("zone_bunker.jpg", img.size)


if __name__ == "__main__":
    main()
