"""Writes the P-1 DXF fixtures for uncad 0.3.0 (see README.md next to this file).

Every file is authored here from scratch (no third-party drawing is copied).
Text is written as raw bytes so the CP949 fixture carries real code-page
bytes, not UTF-8; line endings are CRLF.

    python make_fixtures.py [out_dir] [which]

`which` defaults to `all` (the four shipped DXF files, in their verified
"full" form: the dimension fixture carries a BLOCK_RECORD table so its *D1
block binds, the viewport fixture a *Paper_Space block so the VIEWPORT lands
in paper space). `dimlfac-minimal` and `viewport-minimal` write the earlier
ENTITIES-only variants README.md describes, kept for comparison.
"""
import os
import sys

OUT = sys.argv[1] if len(sys.argv) > 1 and sys.argv[1] else os.path.dirname(os.path.abspath(__file__))


def pair(code, value):
    """One DXF group: right-aligned 3-char code, value, each on its own line."""
    if isinstance(value, bytes):
        v = value
    elif isinstance(value, float):
        v = repr(value).encode("ascii")
    else:
        v = str(value).encode("ascii")
    return b"%3d\r\n" % code + v + b"\r\n"


def pairs(*items):
    return b"".join(pair(c, v) for c, v in items)


def section(name, body):
    return pairs((0, "SECTION"), (2, name)) + body + pairs((0, "ENDSEC"))


def header(acadver="AC1015", **vars_):
    """HEADER section. vars_ maps NAME (without $) to a (code, value) pair."""
    body = pairs((9, "$ACADVER"), (1, acadver))
    for name, spec in vars_.items():
        body += pair(9, "$" + name)
        if isinstance(spec, list):
            body += pairs(*spec)
        else:
            body += pair(*spec)
    return section("HEADER", body)


def entity(kind, layer=b"0", *groups, handle=None, owner=None, paper=False):
    out = pair(0, kind)
    if handle is not None:
        out += pair(5, handle)
    if owner is not None:
        out += pair(330, owner)
    out += pair(8, layer)
    if paper:
        out += pair(67, 1)
    out += pairs(*groups)
    return out


def text(value, x, y, height=2.5, layer=b"0", extrusion=None, handle=None, owner=None):
    g = [(10, float(x)), (20, float(y)), (30, 0.0), (40, float(height)), (1, value)]
    if extrusion is not None:
        g += [(210, float(extrusion[0])), (220, float(extrusion[1])), (230, float(extrusion[2]))]
    return entity("TEXT", layer, *g, handle=handle, owner=owner)


def line(x1, y1, x2, y2, layer=b"0", handle=None, owner=None):
    return entity(
        "LINE", layer,
        (10, float(x1)), (20, float(y1)), (30, 0.0),
        (11, float(x2)), (21, float(y2)), (31, 0.0),
        handle=handle, owner=owner,
    )


def lwpolyline(points, closed, extrusion=None, bulges=None, layer=b"0"):
    g = [(100, "AcDbPolyline"), (90, len(points)), (70, 1 if closed else 0), (43, 0.0)]
    if extrusion is not None:
        g += [(210, float(extrusion[0])), (220, float(extrusion[1])), (230, float(extrusion[2]))]
    for i, (x, y) in enumerate(points):
        g += [(10, float(x)), (20, float(y))]
        if bulges and bulges.get(i):
            g += [(42, float(bulges[i]))]
    return entity("LWPOLYLINE", layer, *g)


def circle(cx, cy, r, extrusion=None, layer=b"0"):
    g = [(10, float(cx)), (20, float(cy)), (30, 0.0), (40, float(r))]
    if extrusion is not None:
        g += [(210, float(extrusion[0])), (220, float(extrusion[1])), (230, float(extrusion[2]))]
    return entity("CIRCLE", layer, *g)


def arc(cx, cy, r, a0, a1, extrusion=None, layer=b"0"):
    g = [(10, float(cx)), (20, float(cy)), (30, 0.0), (40, float(r)), (50, float(a0)), (51, float(a1))]
    if extrusion is not None:
        g += [(210, float(extrusion[0])), (220, float(extrusion[1])), (230, float(extrusion[2]))]
    return entity("ARC", layer, *g)


def tables(layers=((b"0", 7),), dimstyle=False, block_records=()):
    """TABLES section: a LAYER table, optionally DIMSTYLE STANDARD (handle
    30) and a BLOCK_RECORD table whose entries carry explicit handles so
    BLOCK/entity `330` owner codes can refer to them."""
    body = pairs((0, "TABLE"), (2, "LAYER"), (70, len(layers)))
    for name, color in layers:
        body += pairs((0, "LAYER"), (2, name), (70, 0), (62, color), (6, "Continuous"))
    body += pair(0, "ENDTAB")
    if dimstyle:
        body += pairs(
            (0, "TABLE"), (2, "DIMSTYLE"), (70, 1),
            (0, "DIMSTYLE"), (105, "30"), (2, "STANDARD"), (70, 0),
            (0, "ENDTAB"),
        )
    if block_records:
        body += pairs((0, "TABLE"), (2, "BLOCK_RECORD"), (70, len(block_records)))
        for handle, name in block_records:
            body += pairs((0, "BLOCK_RECORD"), (5, handle), (2, name), (70, 0))
        body += pair(0, "ENDTAB")
    return section("TABLES", body)


def block(name, owner, handles, body=b"", paper=False, flag=0):
    """A BLOCK ... ENDBLK pair, owned by BLOCK_RECORD `owner` when given."""
    h_begin, h_end = handles
    begin = entity(
        "BLOCK", b"0",
        (100, "AcDbBlockBegin"), (2, name), (70, flag),
        (10, 0.0), (20, 0.0), (30, 0.0), (3, name),
        handle=h_begin, owner=owner, paper=paper,
    )
    end = entity("ENDBLK", b"0", (100, "AcDbBlockEnd"), handle=h_end, owner=owner, paper=paper)
    return begin + body + end


def write(name, data):
    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(OUT, name)
    with open(path, "wb") as fh:
        fh.write(data)
    print(f"{name}: {len(data)} bytes")


# ---------------------------------------------------------------- fixture 1
def cp949():
    k = lambda s: s.encode("cp949")  # noqa: E731
    # KS X 1001 row 1: A1BE is PLUS-MINUS SIGN (A1B1 is the right double
    # quotation mark); the CP949 codec is the authority here.
    assert k("±") == b"\xa1\xbe", k("±")
    hdr = header(
        DWGCODEPAGE=(3, "ANSI_949"),
        INSUNITS=(70, 4),
        MEASUREMENT=(70, 1),
        LUNITS=(70, 2),
        DIMLFAC=(40, 1.0),
    )
    tbl = tables(layers=((b"0", 7), (k("벽체"), 1)))
    ents = b"".join([
        text(k("도면"), 0, 0, layer=k("벽체")),
        text(k("±3"), 0, 5),
        text(k("32.5㎡"), 0, 10),
        entity(
            "MTEXT", b"0",
            (100, "AcDbMText"),
            (10, 0.0), (20, 20.0), (30, 0.0),
            (40, 2.5), (41, 50.0), (71, 1),
            (1, k("방 101\\P면적 32.5㎡")),
        ),
        text(b"PLAIN", 0, 15),
    ])
    return hdr + tbl + section("ENTITIES", ents) + pair(0, "EOF")


# ---------------------------------------------------------------- fixture 2
def mirrored():
    hdr = header(INSUNITS=(70, 4))
    rect = [(0, 0), (100, 0), (100, 50), (0, 50)]
    ents = b"".join([
        lwpolyline(rect, closed=True, extrusion=(0, 0, -1)),
        lwpolyline(rect, closed=False, extrusion=(0, 0, 1), bulges={1: 0.41421356}),
        circle(10, 10, 5, extrusion=(0, 0, -1)),
        arc(0, 0, 20, 0, 90, extrusion=(0, 0, -1)),
        text(b"MIRROR", 10, 10, height=2.5, extrusion=(0, 0, -1)),
        line(-5, -5, 5, 5),
    ])
    return hdr + section("ENTITIES", ents) + pair(0, "EOF")


# ---------------------------------------------------------------- fixture 3
def dimlfac12(variant="full"):
    hdr = header(
        INSUNITS=(70, 4),
        DIMLFAC=(40, 12.0),
        DIMDEC=(70, 2),
        DIMLUNIT=(70, 2),
        DIMSCALE=(40, 1.0),
    )
    if variant == "full":
        # Verified 2026-09-21: *Model_Space keeps the reader's pre-created handle 1F,
        # *D1 gets 40, and BLOCK/TEXT/ENDBLK point at it through 330.
        pre = tables(dimstyle=True, block_records=(("1F", "*Model_Space"), ("40", "*D1")))
        pre += section("BLOCKS", block(
            "*D1", "40", ("41", "43"), flag=1,
            body=text(b"120", 5, 6, height=0.18, handle="42", owner="40"),
        ))
    else:
        # Shipped: the BLOCK is read but binds to no BLOCK_HEADER (README).
        pre = section("BLOCKS", block(
            "*D1", None, (None, None), flag=1,
            body=text(b"120", 5, 6, height=0.18),
        ))
    dim = entity(
        "DIMENSION", b"0",
        (100, "AcDbDimension"),
        (2, "*D1"),
        (10, 10.0), (20, 5.0), (30, 0.0),
        (11, 5.0), (21, 6.0), (31, 0.0),
        (70, 32),
        (1, b""),
        (42, 10.0),
        (3, "STANDARD"),
        (100, "AcDbAlignedDimension"),
        (13, 0.0), (23, 0.0), (33, 0.0),
        (14, 10.0), (24, 0.0), (34, 0.0),
        (50, 0.0),
        (100, "AcDbRotatedDimension"),
    )
    ents = line(0, 0, 10, 0) + dim
    return hdr + pre + section("ENTITIES", ents) + pair(0, "EOF")


# ---------------------------------------------------------------- fixture 4
def twisted_viewport(variant="full"):
    hdr = header(INSUNITS=(70, 4))
    pre = b""
    owner = None
    line_owner = None
    if variant == "full":
        # Verified 2026-09-21: a *Paper_Space BLOCK_RECORD/BLOCK so that the VIEWPORT's
        # 330 owner equals BLOCK_RECORD_PSPACE and it gets entmode 1.
        pre = tables(block_records=(("1F", "*Model_Space"), ("1C", "*Paper_Space")))
        pre += section("BLOCKS",
                       block("*Model_Space", "1F", ("20", "21"))
                       + block("*Paper_Space", "1C", ("22", "23"), paper=True))
        owner = "1C"
        line_owner = "1F"
    vp = entity(
        "VIEWPORT", b"0",
        (100, "AcDbViewport"),
        (10, 150.0), (20, 100.0), (30, 0.0),
        (40, 200.0), (41, 120.0),
        (68, 1), (69, 2),
        (12, 50.0), (22, 25.0),
        (13, 0.0), (23, 0.0),
        (14, 10.0), (24, 10.0),
        (15, 10.0), (25, 10.0),
        (16, 0.0), (26, 0.0), (36, 1.0),
        (17, 0.0), (27, 0.0), (37, 0.0),
        (42, 50.0), (43, 0.0), (44, 0.0),
        (45, 60.0),
        (50, 0.0),
        (51, 30.0),
        (72, 100),
        (90, 32864),
        handle="2A", owner=owner, paper=True,
    )
    ents = line(0, 0, 100, 50, handle="24" if line_owner else None, owner=line_owner) + vp
    return hdr + pre + section("ENTITIES", ents) + pair(0, "EOF")


if __name__ == "__main__":
    which = sys.argv[2] if len(sys.argv) > 2 else "all"
    if which in ("all", "cp949"):
        write("cp949_r2000.dxf", cp949())
    if which in ("all", "mirrored"):
        write("mirrored_ocs_r2000.dxf", mirrored())
    if which in ("all", "dimlfac"):
        write("dimlfac12_r2000.dxf", dimlfac12("full"))
    if which in ("all", "viewport"):
        write("twisted_viewport_r2000.dxf", twisted_viewport("full"))
    if which == "dimlfac-minimal":
        write("dimlfac12_r2000.dxf", dimlfac12("minimal"))
    if which == "viewport-minimal":
        write("twisted_viewport_r2000.dxf", twisted_viewport("minimal"))
