"""Comparison sheet docs/mockups/home_variants.png: A | B on top, C | notes + update states below, full-size frames.
No "→" in texts: Bahnschrift has no arrows (renders a box).

Run after design/tools/render.ps1 (needs docs/mockups/home_{a,b,c}.png and home_{a,b,c}_update.png).
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]
MOCK = ROOT / "docs" / "mockups"
FONT = "C:/Windows/Fonts/bahnschrift.ttf"
W, H = 1280, 760
GAP, HEAD = 48, 104
BG, INK, DIM, SAND, RULE = (20, 21, 19), (240, 240, 236), (150, 152, 144), (217, 177, 96), (52, 54, 50)

VARIANTS = {
    "a": ("A", "ПДА / полевой терминал",
          "Лаунчер как КПК сталкера: карта Зоны со связью до хоста, изменения — сообщениями, моноширинные показания. "
          "Самый «игровой» и плотный, весь с клавиатуры (F1–F4, Enter)."),
    "b": ("B", "Атмосферный лаунчер · рекомендую",
          "Кадр из игры на весь экран, белая типографика как в HUD v2, разделы — на полосе компаса. "
          "Современный лаунчер уровня Battle.net / Tarkov, но со своим лицом."),
    "c": ("C", "Маркировка",
          "Стенд как промаркированное оборудование: трафаретный номер сборки, паспорт, сигнальный жёлтый, "
          "лента — только там, где идёт работа. Самый смелый и единственный светлый."),
}


def font(size: int, style: str = "Regular") -> ImageFont.FreeTypeFont:
    f = ImageFont.truetype(FONT, size)
    f.set_variation_by_name(style)
    return f


def wrap(draw: ImageDraw.ImageDraw, text: str, f: ImageFont.FreeTypeFont, width: int) -> list[str]:
    lines, cur = [], ""
    for word in text.split():
        nxt = (cur + " " + word).strip()
        if draw.textlength(nxt, font=f) <= width:
            cur = nxt
        else:
            lines.append(cur)
            cur = word
    if cur:
        lines.append(cur)
    return lines


def header(draw: ImageDraw.ImageDraw, x: int, y: int, key: str) -> None:
    letter, name, note = VARIANTS[key]
    draw.rectangle((x, y + 6, x + 52, y + 58), fill=SAND if key == "b" else INK)
    draw.text((x + 26, y + 33), letter, font=font(40, "Bold"), fill=BG, anchor="mm")
    draw.text((x + 72, y + 4), name, font=font(34, "Bold SemiCondensed"), fill=INK)
    f = font(19)
    for i, line in enumerate(wrap(draw, note, f, W - 80)[:2]):
        draw.text((x + 72, y + 48 + i * 25), line, font=f, fill=DIM)


def main() -> None:
    cw, ch = GAP * 3 + W * 2, GAP * 3 + (HEAD + H) * 2
    sheet = Image.new("RGB", (cw, ch), BG)
    d = ImageDraw.Draw(sheet)
    cells = {"a": (GAP, GAP), "b": (GAP * 2 + W, GAP), "c": (GAP, GAP * 2 + HEAD + H)}
    for key, (x, y) in cells.items():
        header(d, x, y, key)
        img = Image.open(MOCK / f"home_{key}.png").convert("RGB")
        sheet.paste(img, (x, y + HEAD))
        d.rectangle((x - 1, y + HEAD - 1, x + W, y + HEAD + H), outline=SAND if key == "b" else RULE, width=1)

    # notes cell: update state of each direction + what is a stand-in in the mockups
    x, y = GAP * 2 + W, GAP * 2 + HEAD + H
    d.text((x, y + 4), "То же окно во время обновления", font=font(34, "Bold SemiCondensed"), fill=INK)
    d.text((x, y + 50), "шаги «качаю · проверяю · ставлю»; обновляется только наш слой, GAMMA не трогаем",
           font=font(19), fill=DIM)
    tw = (W - 2 * 24) // 3
    th = round(tw * H / W)
    for i, key in enumerate("abc"):
        tx = x + i * (tw + 24)
        ty = y + HEAD
        thumb = Image.open(MOCK / f"home_{key}_update.png").convert("RGB").resize((tw, th), Image.LANCZOS)
        sheet.paste(thumb, (tx, ty))
        d.rectangle((tx - 1, ty - 1, tx + tw, ty + th), outline=RULE, width=1)
        d.text((tx, ty + th + 10), VARIANTS[key][0], font=font(24, "Bold"), fill=SAND if key == "b" else INK)
    ny = y + HEAD + th + 64
    notes = [
        "Окно 1280×760 без рамки ОС, своя полоса заголовка. Кадр — GAMMA владельца из макетов HUD v2.",
        "Шрифты в макетах — заглушки: Bahnschrift (Windows) и JetBrains Mono. В приложении — Sofia Sans,",
        "Sofia Sans Condensed и JetBrains Mono: OFL, полная кириллица, файлы лягут в приложение.",
        "Адрес 203.0.113.24 — документационный, вместо адреса друга. Сборка 11 — пример.",
        "HTML: quarknet-launcher/design/home-{a,b,c}.html  ·  бриф: docs/DESIGN.md",
    ]
    for i, line in enumerate(notes):
        d.text((x, ny + i * 30), line, font=font(20), fill=DIM if i < 4 else INK)
    out = MOCK / "home_variants.png"
    sheet.save(out, optimize=True)
    print(out.name, sheet.size, out.stat().st_size // 1024, "KB")


if __name__ == "__main__":
    main()
