"""Writes the P-1 DXF fixtures for uncad 0.3.0 (see README.md next to this file).

Every file is authored here from scratch (no third-party drawing is copied).
Text is written as raw bytes so the CP949 fixture carries real code-page
bytes, not UTF-8; line endings are CRLF.

    python make_fixtures.py [out_dir] [which]

`which` defaults to `all` (the shipped DXF files, in their verified "full"
form: the dimension fixture carries a BLOCK_RECORD table so its *D1 block
binds, the viewport fixture a *Paper_Space block so the VIEWPORT lands in
paper space plus a LAYOUT object with plot settings for that block).
`dimlfac-minimal` and `viewport-minimal` write the earlier ENTITIES-only
variants README.md describes, kept for comparison.
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


def hatch(points, angle_deg, spacing, handle=None, owner=None, paper=False, layer=b"0"):
    """A pattern HATCH (not solid) over the closed polyline `points`: one
    user-defined family of lines at `angle_deg`, `spacing` apart, no dashes.
    The definition line's offset (45/46) is the perpendicular to the line
    direction scaled to `spacing`; the seed point sits one unit inside the
    first vertex."""
    import math
    a = math.radians(angle_deg)
    # Rounded so a right angle writes 0.0, not 1.2e-16 or -0.0.
    ox, oy = round(-math.sin(a) * spacing, 12) + 0.0, round(math.cos(a) * spacing, 12) + 0.0
    g = [
        (100, "AcDbHatch"),
        (10, 0.0), (20, 0.0), (30, 0.0),     # elevation point
        (210, 0.0), (220, 0.0), (230, 1.0),  # extrusion
        (2, "USER"),                         # pattern name
        (70, 0),                             # pattern fill, not solid
        (71, 0),                             # not associative
        (91, 1),                             # one boundary path
        (92, 3),                             # external + polyline
        (72, 0),                             # no bulges
        (73, 1),                             # closed
        (93, len(points)),
    ]
    for x, y in points:
        g += [(10, float(x)), (20, float(y))]
    g += [
        (97, 0),                             # no source boundary objects
        (75, 1),                             # hatch style: outermost
        (76, 0),                             # user-defined pattern
        (52, 0.0),                           # pattern angle
        (41, 1.0),                           # pattern scale
        (77, 0),                             # not double
        (78, 1),                             # one definition line
        (53, float(angle_deg)),
        (43, 0.0), (44, 0.0),                # base point
        (45, ox), (46, oy),                  # offset
        (79, 0),                             # no dash items
        (47, 1.0),                           # pixel size
        (98, 1),                             # one seed point
        (10, float(points[0][0]) + 1.0), (20, float(points[0][1]) + 1.0),
    ]
    return entity("HATCH", layer, *g, handle=handle, owner=owner, paper=paper)


def tables(layers=((b"0", 7),), dimstyle=False, block_records=(), dimlfac=None):
    """TABLES section: a LAYER table, optionally DIMSTYLE STANDARD (handle
    30, with DIMLFAC group 144 when `dimlfac` is given -- a dimension uses
    its style's factor, not the header's) and a BLOCK_RECORD table whose
    entries carry explicit handles so BLOCK/entity `330` owner codes can
    refer to them; a third element on a block record is the handle of its
    LAYOUT object (`340`)."""
    body = pairs((0, "TABLE"), (2, "LAYER"), (70, len(layers)))
    for name, color in layers:
        body += pairs((0, "LAYER"), (2, name), (70, 0), (62, color), (6, "Continuous"))
    body += pair(0, "ENDTAB")
    if dimstyle:
        body += pairs(
            (0, "TABLE"), (2, "DIMSTYLE"), (70, 1),
            (0, "DIMSTYLE"), (105, "30"), (2, "STANDARD"), (70, 0),
            *(((144, dimlfac),) if dimlfac is not None else ()),
            (0, "ENDTAB"),
        )
    if block_records:
        body += pairs((0, "TABLE"), (2, "BLOCK_RECORD"), (70, len(block_records)))
        for handle, name, *layout in block_records:
            body += pairs((0, "BLOCK_RECORD"), (5, handle), (2, name), (70, 0))
            if layout:
                body += pair(340, layout[0])
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


def dictionary(handle, owner, items):
    """A DICTIONARY object (OBJECTS section) whose `items` are (name, handle)
    soft-owner entries (`3` / `350`)."""
    body = pairs((0, "DICTIONARY"), (5, handle), (330, owner), (100, "AcDbDictionary"), (281, 1))
    for name, target in items:
        body += pairs((3, name), (350, target))
    return body


def layout(handle, owner, name, tab_order, block_record, viewport, paper,
           printer="none_device", margin=6.35, rotation=1):
    """A LAYOUT object: the embedded AcDbPlotSettings (page setup, all
    lengths in mm) followed by AcDbLayout. `paper` is (canonical media
    name, width, height) of the unrotated sheet; `rotation` 1 is 90 degrees
    counter-clockwise (landscape), and LIMMIN/LIMMAX are the printable area
    of the sheet as rotated, from (-margin, -margin). EXTMIN/EXTMAX carry
    the 1e20 / -1e20 "never computed" sentinels AutoCAD writes for a layout
    that has not been plotted or zoomed. `owner` is the ACAD_LAYOUT
    dictionary; the AcDbLayout `330` is the block record and `331` the
    active viewport."""
    media, w, h = paper
    if rotation in (1, 3):
        w, h = h, w
    plot = [
        (100, "AcDbPlotSettings"),
        (1, b""),                            # page setup name
        (2, printer),                        # printer / plot configuration
        (4, media),                          # canonical media name
        (40, margin), (41, margin), (42, margin), (43, margin),
        (44, float(paper[1])), (45, float(paper[2])),
        (46, 0.0), (47, 0.0),                # plot origin
        (48, 0.0), (49, 0.0),                # plot window lower-left
        (140, 0.0), (141, 0.0),              # plot window upper-right
        (142, 1.0), (143, 1.0),              # paper units : drawing units
        (70, 688),                           # plot flags
        (72, 1),                             # plot paper unit: mm
        (73, rotation),                      # plot rotation
        (74, 5),                             # plot type: layout
        (7, b""),                            # style sheet
        (75, 16),                            # standard scale: 1:1
        (147, 1.0),                          # standard scale factor
        (148, 0.0), (149, 0.0),              # paper image origin
    ]
    lay = [
        (100, "AcDbLayout"),
        (1, name),
        (70, 1),                             # layout flags: PSLTSCALE
        (71, tab_order),
        (10, -margin), (20, -margin),        # LIMMIN
        (11, w - margin), (21, h - margin),  # LIMMAX
        (12, 0.0), (22, 0.0), (32, 0.0),     # INSBASE
        (14, 1e20), (24, 1e20), (34, 1e20),  # EXTMIN
        (15, -1e20), (25, -1e20), (35, -1e20),  # EXTMAX
        (146, 0.0),                          # elevation
        (13, 0.0), (23, 0.0), (33, 0.0),     # UCSORG
        (16, 1.0), (26, 0.0), (36, 0.0),     # UCSXDIR
        (17, 0.0), (27, 1.0), (37, 0.0),     # UCSYDIR
        (76, 0),                             # UCSORTHOVIEW
        (330, block_record),
        (331, viewport),
    ]
    return pairs((0, "LAYOUT"), (5, handle), (330, owner), *plot, *lay)


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


# ---------------------------------------------------------------- fixture 2b
def mirrored_bulge():
    """The bulged outline of the mirrored fixture (handle 21 there) drawn in
    a mirrored OCS, next to an ARC tracing the same arc in the same OCS: the
    90-degree arc from OCS (100,0) to (100,50) has its centre at (75,25) and
    its apex at (110.355,25), so in the world both curves run from (-100,0)
    to (-100,50) around (-75,25) with the apex at (-110.355,25)."""
    hdr = header(INSUNITS=(70, 4))
    rect = [(0, 0), (100, 0), (100, 50), (0, 50)]
    ents = b"".join([
        lwpolyline(rect, closed=False, extrusion=(0, 0, -1), bulges={1: 0.41421356}),
        arc(75, 25, 35.35533906, 315, 45, extrusion=(0, 0, -1)),
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
        pre = tables(dimstyle=True, block_records=(("1F", "*Model_Space"), ("40", "*D1")), dimlfac=12.0)
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
    post = b""
    owner = None
    line_owner = None
    if variant == "full":
        # Verified 2026-09-21: a *Paper_Space BLOCK_RECORD/BLOCK so that the VIEWPORT's
        # 330 owner equals BLOCK_RECORD_PSPACE and it gets entmode 1.
        pre = tables(block_records=(("1F", "*Model_Space"), ("1C", "*Paper_Space", "2B")))
        pre += section("BLOCKS",
                       block("*Model_Space", "1F", ("20", "21"))
                       + block("*Paper_Space", "1C", ("22", "23"), paper=True))
        owner = "1C"
        line_owner = "1F"
        # Verified 2026-09-22: an OBJECTS section with the named object
        # dictionary (C), its ACAD_LAYOUT dictionary (1A) and one LAYOUT (2B)
        # for *Paper_Space, an A4 sheet in landscape. LibreDWG builds the
        # LAYOUT from the object alone; the dictionaries are what lets it set
        # HEADER.DICTIONARY_LAYOUT (README.md). No Model layout.
        post = section("OBJECTS",
                       dictionary("C", "0", (("ACAD_LAYOUT", "1A"),))
                       + dictionary("1A", "C", (("Layout1", "2B"),))
                       + layout("2B", "1A", "Layout1", 1, block_record="1C", viewport="2A",
                                paper=("ISO_A4_(210.00_x_297.00_MM)", 210.0, 297.0)))
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
    return hdr + pre + section("ENTITIES", ents) + post + pair(0, "EOF")


def hatched_viewport():
    """The twisted-viewport fixture (its full form: tables, blocks, the
    LAYOUT) with a pattern HATCH in each space: vertical lines 2 units apart
    over model (20,10)-(60,30), under the viewport, and horizontal lines 4
    units apart over paper (10,10)-(40,30), outside the viewport's frame
    (x 50..250, y 40..160). Both hatches are the first pattern of their
    render, so a composited sheet that did not namespace its <defs> would
    define `hp0` twice."""
    hdr = header(INSUNITS=(70, 4))
    pre = tables(block_records=(("1F", "*Model_Space"), ("1C", "*Paper_Space", "2B")))
    pre += section("BLOCKS",
                   block("*Model_Space", "1F", ("20", "21"))
                   + block("*Paper_Space", "1C", ("22", "23"), paper=True))
    post = section("OBJECTS",
                   dictionary("C", "0", (("ACAD_LAYOUT", "1A"),))
                   + dictionary("1A", "C", (("Layout1", "2B"),))
                   + layout("2B", "1A", "Layout1", 1, block_record="1C", viewport="2A",
                            paper=("ISO_A4_(210.00_x_297.00_MM)", 210.0, 297.0)))
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
        handle="2A", owner="1C", paper=True,
    )
    ents = (line(0, 0, 100, 50, handle="24", owner="1F")
            + hatch([(20, 10), (60, 10), (60, 30), (20, 30)], 90.0, 2.0, handle="30", owner="1F")
            + vp
            + hatch([(10, 10), (40, 10), (40, 30), (10, 30)], 0.0, 4.0, handle="31", owner="1C", paper=True))
    return hdr + pre + section("ENTITIES", ents) + post + pair(0, "EOF")


def hidden_layers():
    """Every way an entity can be hidden: a LINE on each of the layers below
    (one per 10 units of y), plus an invisible LINE (DXF 60 = 1) and a
    0.50 mm DASHED one (370 = 50, 6 = DASHED, 48 = 2.0) on VISIBLE."""
    hdr = header(INSUNITS=(70, 4))
    # (name, colour, DXF 70 flag, DXF 290 plot flag or None to omit)
    layers = [
        (b"0", 7, 0, None),
        (b"VISIBLE", 1, 0, None),
        (b"OFF", -3, 0, None),      # negative colour: the layer is off
        (b"FROZEN", 4, 1, None),    # 70 bit 1: frozen
        (b"NOPLOT", 5, 0, 0),       # 290 = 0: not plotted
        (b"Defpoints", 7, 0, 0),    # AutoCAD's own non-plotting layer
        (b"LOCKED", 6, 4, None),    # 70 bit 4: locked (still drawn)
    ]
    body = pairs((0, "TABLE"), (2, "LTYPE"), (70, 2))
    body += pairs(
        (0, "LTYPE"), (5, "14"), (100, "AcDbSymbolTableRecord"), (100, "AcDbLinetypeTableRecord"),
        (2, "Continuous"), (70, 0), (3, "Solid line"), (72, 65), (73, 0), (40, 0.0),
    )
    body += pairs(
        (0, "LTYPE"), (5, "15"), (100, "AcDbSymbolTableRecord"), (100, "AcDbLinetypeTableRecord"),
        (2, "DASHED"), (70, 0), (3, "Dashed __ __ __"), (72, 65), (73, 2), (40, 0.75),
        (49, 0.5), (74, 0), (49, -0.25), (74, 0),
    )
    body += pair(0, "ENDTAB")
    body += pairs((0, "TABLE"), (2, "LAYER"), (70, len(layers)))
    for name, color, flag, plot in layers:
        body += pairs((0, "LAYER"), (2, name), (70, flag), (62, color), (6, "Continuous"))
        if plot is not None:
            body += pair(290, plot)
    body += pair(0, "ENDTAB")
    tbl = section("TABLES", body)
    ents = b""
    for i, (name, _color, _flag, _plot) in enumerate(layers):
        ents += line(0, 10 * i, 100, 10 * i, layer=name)
    ents += entity(
        "LINE", b"VISIBLE", (100, "AcDbEntity"), (60, 1), (100, "AcDbLine"),
        (10, 0.0), (20, 100.0), (30, 0.0), (11, 100.0), (21, 100.0), (31, 0.0),
    )
    ents += entity(
        "LINE", b"VISIBLE", (100, "AcDbEntity"), (6, "DASHED"), (370, 50), (48, 2.0), (100, "AcDbLine"),
        (10, 0.0), (20, 110.0), (30, 0.0), (11, 100.0), (21, 110.0), (31, 0.0),
    )
    return hdr + tbl + section("ENTITIES", ents) + pair(0, "EOF")


if __name__ == "__main__":
    which = sys.argv[2] if len(sys.argv) > 2 else "all"
    if which in ("all", "cp949"):
        write("cp949_r2000.dxf", cp949())
    if which in ("all", "mirrored"):
        write("mirrored_ocs_r2000.dxf", mirrored())
    if which in ("all", "mirrored-bulge"):
        write("mirrored_bulge_r2000.dxf", mirrored_bulge())
    if which in ("all", "dimlfac"):
        write("dimlfac12_r2000.dxf", dimlfac12("full"))
    if which in ("all", "viewport"):
        write("twisted_viewport_r2000.dxf", twisted_viewport("full"))
    if which in ("all", "hidden"):
        write("hidden_layers_r2000.dxf", hidden_layers())
    if which in ("all", "hatched-viewport"):
        write("hatched_viewport_r2000.dxf", hatched_viewport())
    if which == "dimlfac-minimal":
        write("dimlfac12_r2000.dxf", dimlfac12("minimal"))
    if which == "viewport-minimal":
        write("twisted_viewport_r2000.dxf", twisted_viewport("minimal"))
