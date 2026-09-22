# Bundled font: Uncad Sans

`UncadSans-Regular.otf` (370 KB) is a subset of **Noto Sans KR Regular
v2.004** (Adobe / Google, SIL Open Font License 1.1 -- `OFL-NotoSansKR.txt`
is the licence with its copyright notice). It is what `to_png`, the tiles
and the text metrics pre-pass shape text with, so a drawing renders the
same on every machine and a Hangul label never depends on the host's fonts.

Coverage (2755 glyphs): Basic Latin, Latin-1 Supplement, the part of Latin
Extended-A Noto Sans KR has, Greek, General Punctuation, Hangul Compatibility
Jamo, the 2350 KS X 1001 Hangul syllables, and the CAD symbols `∅ ° ± ² ³ ×
Ø ㎡ ㎜ ㎥ ㎝ ㎞ ← ↑ → ↓ φ Δ Ω`. Hanja, the other 8822 syllables and `⌀`
(U+2300, absent from Noto Sans KR itself) are not included: with the default
`Fonts::Bundled` such a character is drawn as a crossed `.notdef` box (the
subset keeps the notdef outline, see below) and counted in the export's
`unshaped_glyphs` / `UnshapedGlyphs` warning; with `Fonts::BundledAndSystem`
the host's fonts fill in.

## Why this font

Only Noto Sans KR and Pretendard cover every block above (NanumGothic, IBM
Plex Sans KR and Spoqa Han Sans lack `Ø`, `∅`, `㎡` or Greek; Gowun Dodum
subsets to 1.2 MB). Noto is the canonical, most widely mirrored face and
its OFL Reserved Font Name is `Source`, which the subset's name does not
use. The subset is renamed to `Uncad Sans` anyway: the OFL asks a Modified
Version not to pass as the original, and a unique family name keeps fontdb
from picking a host-installed Noto Sans KR over the bundled face.

## Reproducing the file

Source: https://github.com/notofonts/noto-cjk/raw/main/Sans/SubsetOTF/KR/NotoSansKR-Regular.otf
(4,644,748 bytes). Tools: Python 3 with `fonttools` and `brotli`
(`uv venv venv && uv pip install --python venv/Scripts/python.exe fonttools brotli`).

```text
python tools/gen_unicodes.py                        # rewrites tools/unicodes.txt (2365 lines)
set PYTHONUTF8=1
pyftsubset NotoSansKR-Regular.otf --unicodes-file=tools/unicodes.txt \
    --output-file=NotoSansKR-Regular-sub.otf --name-IDs='*' --notdef-outline --layout-features=''
python tools/rename.py NotoSansKR-Regular-sub.otf UncadSans-Regular.otf "Uncad Sans" UncadSans-Regular
```

`gen_unicodes.py` writes next to itself, whatever directory it is run
from, so it rewrites the `tools/unicodes.txt` checked in here -- the file
`pyftsubset` reads -- and the rewrite is byte for byte the same file.

`--layout-features=''` drops kerning and the other GSUB/GPOS tables (16 KB
of kerning would keep glyph spacing identical to full Noto; the metrics
pre-pass measures whatever is bundled, so records and pixels agree either
way). `--name-IDs='*'` keeps the copyright and licence name records; the
rename script changes the family, full and PostScript names and appends
`uncad subset` to the version string, as the OFL asks of a Modified Version.

## Licence

The font is OFL 1.1 (not GPL like the rest of this repository). Bundling it
in GPL software is permitted by the OFL; it may not be sold by itself, and
the licence text must travel with it -- `cargo package` includes this
directory, and every binary linking `uncad` embeds the font bytes.
