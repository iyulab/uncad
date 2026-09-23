"""Writes the DXF test fixtures (see README.md next to this file).

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


def text(value, x, y, height=2.5, layer=b"0", extrusion=None, handle=None, owner=None,
         paper=False):
    g = [(10, float(x)), (20, float(y)), (30, 0.0), (40, float(height)), (1, value)]
    if extrusion is not None:
        g += [(210, float(extrusion[0])), (220, float(extrusion[1])), (230, float(extrusion[2]))]
    return entity("TEXT", layer, *g, handle=handle, owner=owner, paper=paper)


def line(x1, y1, x2, y2, layer=b"0", handle=None, owner=None, paper=False,
         color=None, true_color=None):
    """A LINE. `color` is the DXF 62 ACI index (0 BYBLOCK, 256 BYLAYER) and
    `true_color` the DXF 420 packed RGB, written in that order -- which is
    the order AutoCAD writes them and the one LibreDWG's reader needs, since
    a later 62 overwrites the colour a 420 stored."""
    color_groups = []
    if color is not None:
        color_groups.append((62, color))
    if true_color is not None:
        color_groups.append((420, true_color))
    return entity(
        "LINE", layer, *color_groups,
        (10, float(x1)), (20, float(y1)), (30, 0.0),
        (11, float(x2)), (21, float(y2)), (31, 0.0),
        handle=handle, owner=owner, paper=paper,
    )


def polyline(subclass, vertex_subclass, vertices, groups=(), vertex_flag=None,
             layer=b"0", handle=None, owner=None, vertex_handles=(), seqend=None,
             vertex_owner=None, faces=()):
    """An old-style POLYLINE with its VERTEX chain and SEQEND -- the shape
    every DXF uses for a 2D/3D polyline, a polyface mesh and a polygon mesh
    (they differ only in the two subclass markers and the group 70 flags).

    `vertices` are (x, y, z) or (x, y, z, bulge); `faces` are extra VERTEX
    records holding 1-based vertex indices (`AcDbFaceRecord`, group 71-74).
    `vertex_owner` is what the VERTEX records name in group 330: AutoCAD
    writes the POLYLINE's own handle there, ezdxf writes the block record,
    and LibreDWG's DXF reader picks a polyface vertex's *type* from it."""
    g = [(100, "AcDbEntity"), (100, subclass), (66, 1),
         (10, 0.0), (20, 0.0), (30, 0.0)]
    g += list(groups)
    out = entity("POLYLINE", layer, *g, handle=handle, owner=owner)
    vowner = vertex_owner if vertex_owner is not None else handle
    for i, v in enumerate(vertices):
        vg = [(100, "AcDbEntity"), (100, "AcDbVertex"), (100, vertex_subclass),
              (10, float(v[0])), (20, float(v[1])), (30, float(v[2]))]
        if len(v) > 3:
            vg.append((42, float(v[3])))
        if vertex_flag is not None:
            vg.append((70, vertex_flag))
        out += entity("VERTEX", layer, *vg,
                      handle=vertex_handles[i] if vertex_handles else None,
                      owner=vowner)
    for i, face in enumerate(faces):
        fg = [(100, "AcDbEntity"), (100, "AcDbFaceRecord"),
              (10, 0.0), (20, 0.0), (30, 0.0), (70, 128)]
        fg += [(71 + j, int(idx)) for j, idx in enumerate(face)]
        out += entity("VERTEX", layer, *fg,
                      handle=vertex_handles[len(vertices) + i] if vertex_handles else None,
                      owner=vowner)
    out += entity("SEQEND", layer, (100, "AcDbEntity"), handle=seqend, owner=handle)
    return out


def lwpolyline(points, closed, extrusion=None, bulges=None, layer=b"0",
               handle=None, owner=None, paper=False):
    g = [(100, "AcDbPolyline"), (90, len(points)), (70, 1 if closed else 0), (43, 0.0)]
    if extrusion is not None:
        g += [(210, float(extrusion[0])), (220, float(extrusion[1])), (230, float(extrusion[2]))]
    for i, (x, y) in enumerate(points):
        g += [(10, float(x)), (20, float(y))]
        if bulges and bulges.get(i):
            g += [(42, float(bulges[i]))]
    return entity("LWPOLYLINE", layer, *g, handle=handle, owner=owner, paper=paper)


def construction(kind, x, y, dx, dy, layer=b"0", handle=None, owner=None):
    """A RAY or an XLINE: base point (10) and unit direction (11)."""
    return entity(
        kind, layer,
        (100, "AcDbXline" if kind == "XLINE" else "AcDbRay"),
        (10, float(x)), (20, float(y)), (30, 0.0),
        (11, float(dx)), (21, float(dy)), (31, 0.0),
        handle=handle, owner=owner,
    )


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


def viewport(handle, owner, center, size, view_center, view_size, twist_deg=0.0,
             vp_id=2, on=1, status=32864, view_dir=(0.0, 0.0, 1.0), layer=b"0",
             lens=50.0):
    """A paper-space VIEWPORT (`AcDbViewport`). `center`/`size` are the
    frame on the sheet in paper units, `view_center` (DXF 12, VIEWCTR) and
    `view_size` (45, VIEWSIZE) the model window it shows, so the scale is
    `size[1] / view_size`; `twist_deg` is DXF 51 (the reader converts it to
    radians). `on` is DXF 68 (0 switches the viewport off in a DXF) and
    `status` DXF 90, whose 0x20000 bit is a DWG's own "off"; `vp_id` is
    DXF 69 (1 marks the sheet's overall viewport). `view_dir` is DXF 16,
    VIEWDIR -- anything but (0,0,1) is a non-plan view."""
    cx, cy = center
    w, h = size
    vx, vy = view_center
    dx, dy, dz = view_dir
    return entity(
        "VIEWPORT", layer,
        (100, "AcDbViewport"),
        (10, float(cx)), (20, float(cy)), (30, 0.0),
        (40, float(w)), (41, float(h)),
        (68, on), (69, vp_id),
        (12, float(vx)), (22, float(vy)),
        (13, 0.0), (23, 0.0),
        (14, 10.0), (24, 10.0),
        (15, 10.0), (25, 10.0),
        (16, float(dx)), (26, float(dy)), (36, float(dz)),
        (17, 0.0), (27, 0.0), (37, 0.0),
        (42, float(lens)), (43, 0.0), (44, 0.0),
        (45, float(view_size)),
        (50, 0.0),
        (51, float(twist_deg)),
        (72, 100),
        (90, status),
        handle=handle, owner=owner, paper=True,
    )


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
    LAYOUT object (`340`). A layer is `(name, colour)` or
    `(name, colour, flag)` with the DXF 70 flag (1 frozen, 4 locked); a
    negative colour means the layer is off."""
    body = pairs((0, "TABLE"), (2, "LAYER"), (70, len(layers)))
    for name, color, *rest in layers:
        flag = rest[0] if rest else 0
        body += pairs((0, "LAYER"), (2, name), (70, flag), (62, color), (6, "Continuous"))
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
           printer="none_device", margin=6.35, rotation=1, margins=None,
           plot_origin=(0.0, 0.0), paper_units=1):
    """A LAYOUT object: the embedded AcDbPlotSettings (page setup, all
    lengths in mm) followed by AcDbLayout. `paper` is (canonical media
    name, width, height) of the unrotated sheet; `rotation` 1 is 90 degrees
    counter-clockwise (landscape). `margins` is (left, bottom, right, top)
    in mm (default: `margin` on every side), `plot_origin` the DXF 46/47
    offset in mm and `paper_units` DXF 72 (1 mm, 0 inches: the layout's own
    unit, which LIMMIN/LIMMAX are written in). LIMMIN/LIMMAX follow
    AutoCAD's placement of the sheet: the layout origin is the printable
    corner moved by the plot origin, so the paper runs from
    -(margin + origin) to that plus the rotated size (ezdxf's
    `reset_paper_limits`; verified against seven AutoCAD-written layouts on
    2026-09-22). EXTMIN/EXTMAX carry the 1e20 / -1e20 "never computed"
    sentinels AutoCAD writes for a layout that has not been plotted or
    zoomed. `owner` is the ACAD_LAYOUT dictionary; the AcDbLayout `330` is
    the block record and `331` the active viewport."""
    media, w, h = paper
    if rotation in (1, 3):
        w, h = h, w
    left, bottom, right, top = margins if margins is not None else (margin,) * 4
    ox, oy = plot_origin
    k = 1.0 / 25.4 if paper_units == 0 else 1.0
    shift_x, shift_y = left + ox, bottom + oy
    r = lambda v: round(v, 9) + 0.0  # noqa: E731  -- no float noise, no -0.0
    plot = [
        (100, "AcDbPlotSettings"),
        (1, b""),                            # page setup name
        (2, printer),                        # printer / plot configuration
        (4, media),                          # canonical media name
        (40, float(left)), (41, float(bottom)), (42, float(right)), (43, float(top)),
        (44, float(paper[1])), (45, float(paper[2])),
        (46, float(ox)), (47, float(oy)),    # plot origin
        (48, 0.0), (49, 0.0),                # plot window lower-left
        (140, 0.0), (141, 0.0),              # plot window upper-right
        (142, 1.0), (143, 1.0),              # paper units : drawing units
        (70, 688),                           # plot flags
        (72, paper_units),                   # plot paper unit: 1 mm, 0 in
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
        (10, r(-shift_x * k)), (20, r(-shift_y * k)),            # LIMMIN
        (11, r((w - shift_x) * k)), (21, r((h - shift_y) * k)),  # LIMMAX
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
    vp = viewport("2A", owner, (150.0, 100.0), (200.0, 120.0), (50.0, 25.0), 60.0,
                  twist_deg=30.0)
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
    vp = viewport("2A", "1C", (150.0, 100.0), (200.0, 120.0), (50.0, 25.0), 60.0,
                  twist_deg=30.0)
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


# ---------------------------------------------------------------- fixture 8
def attdef(tag, prompt, default, x, y, height, handle, owner):
    """An ATTDEF: the attribute *template* a block definition carries."""
    return entity(
        "ATTDEF", b"0",
        (100, "AcDbEntity"), (100, "AcDbText"),
        (10, float(x)), (20, float(y)), (30, 0.0), (40, float(height)), (1, default),
        (100, "AcDbAttributeDefinition"), (3, prompt), (2, tag), (70, 0),
        handle=handle, owner=owner,
    )


def attrib(tag, value, x, y, height, handle, owner):
    """An ATTRIB: one INSERT's value for a tag. `owner` (DXF 330) decides the
    shape LibreDWG builds -- the owning INSERT links it into that INSERT's
    attribute chain, the block record makes it a child of the block."""
    return entity(
        "ATTRIB", b"0",
        (100, "AcDbEntity"), (100, "AcDbText"),
        (10, float(x)), (20, float(y)), (30, 0.0), (40, float(height)), (1, value),
        (100, "AcDbAttribute"), (2, tag), (70, 0),
        handle=handle, owner=owner,
    )


def insert(name, x, y, handle, owner, attribs=None, seqend=None, layer=b"0", paper=False):
    """An INSERT, with its ATTRIB chain (DXF 66 = 1) and SEQEND when given."""
    groups = [(100, "AcDbEntity"), (100, "AcDbBlockReference")]
    if attribs:
        groups.append((66, 1))
    groups += [(2, name), (10, float(x)), (20, float(y)), (30, 0.0)]
    out = entity("INSERT", layer, *groups, handle=handle, owner=owner, paper=paper)
    if attribs:
        out += attribs
        out += entity("SEQEND", layer, (100, "AcDbEntity"), handle=seqend,
                      owner=owner, paper=paper)
    return out


def nested_attrib():
    """An attributed block inside another block -- the tag-inside-assembly
    pattern -- so the export has to find a nested INSERT's attribute value.

    Block TAG is a LINE and an ATTDEF `NUM`; block DOOR is two LINEs and an
    INSERT of TAG at (20, 20) whose ATTRIB `NUM = D-101` is owned (DXF 330)
    by the *block record*, the shape ezdxf- and AutoCAD-written DXFs use and
    the one LibreDWG turns into a child of DOOR. Model space holds INSERT
    `60` of DOOR at (100, 100) -- so the nested value is drawn at
    (100+21, 100+21) and belongs to the text id `60/56` -- and INSERT `61`
    of TAG at (0, 0) with its own top-level ATTRIB `NUM = D-TOP`, the
    control that always worked."""
    hdr = header(INSUNITS=(70, 4))
    pre = tables(block_records=(("1F", "*Model_Space"), ("40", "TAG"), ("50", "DOOR")))
    tag_body = (line(0, 0, 10, 0, handle="42", owner="40")
                + attdef("NUM", "Number", b"D-000", 1, 1, 2.5, "43", "40"))
    door_body = (line(0, 0, 40, 0, handle="52", owner="50")
                 + line(0, 0, 0, 40, handle="53", owner="50")
                 + insert("TAG", 20, 20, "55", "50",
                          attribs=attrib("NUM", b"D-101", 21, 21, 2.5, "56", "50"),
                          seqend="57"))
    pre += section("BLOCKS",
                   block("TAG", "40", ("41", "44"), body=tag_body)
                   + block("DOOR", "50", ("51", "54"), body=door_body))
    ents = insert("DOOR", 100, 100, "60", "1F")
    ents += insert("TAG", 0, 0, "61", "1F",
                   attribs=attrib("NUM", b"D-TOP", 1, 1, 2.5, "62", "1F"),
                   seqend="63")
    return hdr + pre + section("ENTITIES", ents) + pair(0, "EOF")


# ---------------------------------------------------------------- fixture 6
def plot_origin():
    """A paper layout with the page setup AutoCAD-written drawings usually
    carry: inch units, asymmetric margins and a plot origin that is not
    zero, so the sheet is not at `(-left, -bottom)`. ANSI B landscape
    (431.8 x 279.4 mm = 17 x 11 in) unrotated, margins (0.25, 0.75, 0.25,
    0.75) in, plot origin (-0.25, -0.5) in: the layout origin is the
    printable corner moved by the origin, so the sheet runs from
    (-(0.25 - 0.25), -(0.75 - 0.5)) = (0, -0.25) to (17, 10.75), which is
    what LIMMIN/LIMMAX say. On paper: a border rectangle (0.5, 0.25) ..
    (16.5, 10.5) -- its top edge lies above the 10.25 a margins-only sheet
    would end at -- and a 12 x 8 in plan viewport at (8.5, 5.5) showing the
    model at 1:5 (VIEWSIZE 40 for a height of 8) centred on (50, 25). The
    model holds one LINE (0,0) -> (100,50). Same block/dictionary layout and
    handles as the twisted-viewport fixture."""
    hdr = header(INSUNITS=(70, 1))
    pre = tables(block_records=(("1F", "*Model_Space"), ("1C", "*Paper_Space", "2B")))
    pre += section("BLOCKS",
                   block("*Model_Space", "1F", ("20", "21"))
                   + block("*Paper_Space", "1C", ("22", "23"), paper=True))
    post = section("OBJECTS",
                   dictionary("C", "0", (("ACAD_LAYOUT", "1A"),))
                   + dictionary("1A", "C", (("Layout1", "2B"),))
                   + layout("2B", "1A", "Layout1", 1, block_record="1C", viewport="2A",
                            paper=("ANSI_B_(17.00_x_11.00_Inches)", 431.8, 279.4),
                            rotation=0, margins=(6.35, 19.05, 6.35, 19.05),
                            plot_origin=(-6.35, -12.7), paper_units=0))
    vp = viewport("2A", "1C", (8.5, 5.5), (12.0, 8.0), (50.0, 25.0), 40.0)
    border = lwpolyline([(0.5, 0.25), (16.5, 0.25), (16.5, 10.5), (0.5, 10.5)], closed=True,
                        handle="25", owner="1C", paper=True)
    ents = line(0, 0, 100, 50, handle="24", owner="1F") + border + vp
    return hdr + pre + section("ENTITIES", ents) + post + pair(0, "EOF")


# ---------------------------------------------------------------- fixture 7
def angular_ordinate():
    """The two dimension kinds whose definition points a DXF lays out
    differently from a DWG (LibreDWG maps DXF groups by code, its DWG
    decoder by stream order): a 2-line angular dimension and an ordinate
    dimension of each type. Every value below is derived by hand.

    The angular dimension (AcDb2LineAngularDimension, 70 = 2 | 32): line 1
    from 13 = (0,0) to 14 = (10,0), line 2 from 15 = (0,0) to 10 = (5,
    8.660254) -- for this kind group 10 is the second line's end point --
    and the arc point 16 = (4.330127, 2.5), 30 degrees along a radius of
    5, inside the 60-degree sector; 42 = pi/3. Read with 10 and 16 swapped
    the probe would sit at 60 degrees between rays at 30 and 180 degrees,
    i.e. 150.

    The ordinates (AcDbOrdinateDimension) share the datum origin 10 =
    (100, 200) and the feature 13 = (130, 250); 14 is the leader end. The
    first has bit 64 of 70 set (70 = 6 | 32 | 64 = 102): an X ordinate,
    130 - 100 = 30 (42 = 30.0). The second has 70 = 38: a Y ordinate,
    250 - 200 = 50 (42 = 50.0). No cached *D blocks, so the labels are
    formatted from the values: DIMADEC 0 and DIMDEC 2 from the header."""
    hdr = header(INSUNITS=(70, 4), DIMDEC=(70, 2), DIMADEC=(70, 0), DIMLUNIT=(70, 2))
    tbl = tables(dimstyle=True)
    common = lambda block: [(100, "AcDbDimension"), (2, block)]  # noqa: E731
    angular = entity(
        "DIMENSION", b"0",
        *common("*D1"),
        (10, 5.0), (20, 8.660254037844386), (30, 0.0),
        (11, 6.0), (21, 3.5), (31, 0.0),
        (70, 34),
        (1, b""),
        (42, 1.0471975511965976),
        (3, "STANDARD"),
        (100, "AcDb2LineAngularDimension"),
        (13, 0.0), (23, 0.0), (33, 0.0),
        (14, 10.0), (24, 0.0), (34, 0.0),
        (15, 0.0), (25, 0.0), (35, 0.0),
        (16, 4.330127018922194), (26, 2.5), (36, 0.0),
    )

    def ordinate(block, flag, value, leader):
        return entity(
            "DIMENSION", b"0",
            *common(block),
            (10, 100.0), (20, 200.0), (30, 0.0),
            (11, float(leader[0])), (21, float(leader[1])), (31, 0.0),
            (70, flag),
            (1, b""),
            (42, float(value)),
            (3, "STANDARD"),
            (100, "AcDbOrdinateDimension"),
            (13, 130.0), (23, 250.0), (33, 0.0),
            (14, float(leader[0])), (24, float(leader[1])), (34, 0.0),
        )

    ents = (line(0, 0, 10, 0) + line(0, 0, 5, 8.660254037844386) + angular
            + ordinate("*D2", 102, 30.0, (130, 270))
            + ordinate("*D3", 38, 50.0, (150, 250)))
    return hdr + tbl + section("ENTITIES", ents) + pair(0, "EOF")


# ---------------------------------------------------------------- fixture 8
def viewport_states():
    """Every viewport state the sheet compositing rules distinguish, and two
    page setups the other layout fixtures do not carry.

    `Layout1` (`*Paper_Space`, tab 1): A4 210 x 297 mm with plot rotation 2
    (upside down, so the sheet keeps its portrait size), asymmetric margins
    (10, 20, 5, 15) mm and no plot origin, so the sheet runs from
    (-10, -20) to (200, 277) mm. On it, four 60 x 40 viewports, each
    showing the model window 30 x 20 around (50, 25) at scale 40 / 20 = 2:

    - `2A` at (50, 50): on, plan, layer 0 -- composited, border drawn.
    - `2D` at (50, 120): the same, but on the frozen layer `VPFROZEN`, the
      usual way to hide a viewport's border -- composited, no border.
    - `2E` at (50, 190): switched off (DXF 68 = 0 and the status flag's
      0x20000 bit, 32864 | 0x20000 = 163936) -- not composited.
    - `2F` at (140, 50): VIEWDIR (1,1,1), an isometric view -- not
      composited (the compositing transform is a plan-view mapping).

    `Layout2` (`*Paper_Space0`, tab 2, empty): ANSI B 431.8 x 279.4 mm with
    plot rotation 3 (90 degrees clockwise: the landscape sheet is turned
    portrait) in inch paper units and uniform 6.35 mm margins, so the sheet
    is (-0.25, -0.25) to (273.05 / 25.4, 425.45 / 25.4) = (10.75, 16.75)
    in. It pins the rotation swap and the 1/25.4 conversion in
    `PlotSettings::sheet_rect` against the limits AutoCAD would store.

    The model holds the LINE (0,0) -> (100,50) of the viewport fixtures, so
    each composited window shows it running corner to corner.
    """
    hdr = header(INSUNITS=(70, 4))
    pre = tables(layers=((b"0", 7), (b"VPFROZEN", 4, 1)),
                 block_records=(("1F", "*Model_Space"),
                                ("1C", "*Paper_Space", "2B"),
                                ("1D", "*Paper_Space0", "2C")))
    pre += section("BLOCKS",
                   block("*Model_Space", "1F", ("20", "21"))
                   + block("*Paper_Space", "1C", ("22", "23"), paper=True)
                   + block("*Paper_Space0", "1D", ("26", "27"), paper=True))
    post = section("OBJECTS",
                   dictionary("C", "0", (("ACAD_LAYOUT", "1A"),))
                   + dictionary("1A", "C", (("Layout1", "2B"), ("Layout2", "2C")))
                   + layout("2B", "1A", "Layout1", 1, block_record="1C", viewport="2A",
                            paper=("ISO_A4_(210.00_x_297.00_MM)", 210.0, 297.0),
                            rotation=2, margins=(10.0, 20.0, 5.0, 15.0))
                   + layout("2C", "1A", "Layout2", 2, block_record="1D", viewport="0",
                            paper=("ANSI_B_(17.00_x_11.00_Inches)", 431.8, 279.4),
                            rotation=3, paper_units=0))
    window = dict(view_center=(50.0, 25.0), view_size=20.0)
    ents = (line(0, 0, 100, 50, handle="24", owner="1F")
            + viewport("2A", "1C", (50.0, 50.0), (60.0, 40.0), vp_id=2, **window)
            + viewport("2D", "1C", (50.0, 120.0), (60.0, 40.0), vp_id=3,
                       layer=b"VPFROZEN", **window)
            + viewport("2E", "1C", (50.0, 190.0), (60.0, 40.0), vp_id=4,
                       on=0, status=32864 | 0x20000, **window)
            + viewport("2F", "1C", (140.0, 50.0), (60.0, 40.0), vp_id=5,
                       view_dir=(1.0, 1.0, 1.0), **window))
    return hdr + pre + section("ENTITIES", ents) + post + pair(0, "EOF")


# ---------------------------------------------------------------- fixture 9
def radial():
    """The three dimension kinds no corpus DXF carries (`2000/TS1.dwg` has
    all three, but every TS1 DXF fails LibreDWG's reader): a radius, a
    diameter and a 3-point angular dimension, with no cached `*D` blocks so
    the labels are formatted from the values. Every number is derived by
    hand, and group 42 (`act_measurement`) is written as the value a CAD
    program would have measured, so `measurement` and
    `measurement_from_points` must agree.

    | 70 | subclass | 10 | 13 | 14 | 15 | 42 | value |
    |---|---|---|---|---|---|---|---|
    | 36 (4 radius) | `AcDbRadialDimension` | (0,0) centre | | | (3,4) on the circle | 5.0 | 3-4-5: radius 5 |
    | 35 (3 diameter) | `AcDbDiametricDimension` | (20,0) chord start | | | (20,10) chord end | 10.0 | the vertical diameter of the circle at (20,5), r = 5 |
    | 37 (5 angular 3-point) | `AcDb3PointAngularDimension` | (44.330127, 2.5) arc point, 30 degrees along r = 5 | (50,0) | (45, 8.660254) | (40,0) centre | pi/3 | the 60-degree sector between the rays at 0 and 60 degrees |

    The arc point picks the sector: it lies at 30 degrees, inside the
    60-degree one, so the 300-degree sector on the other side is not the
    one dimensioned.
    """
    hdr = header(INSUNITS=(70, 4), DIMDEC=(70, 2), DIMADEC=(70, 0), DIMLUNIT=(70, 2))
    tbl = tables(dimstyle=True)
    radius = entity(
        "DIMENSION", b"0",
        (100, "AcDbDimension"), (2, "*D1"),
        (10, 0.0), (20, 0.0), (30, 0.0),
        (11, 1.5), (21, 2.0), (31, 0.0),
        (70, 36),
        (1, b""),
        (42, 5.0),
        (3, "STANDARD"),
        (100, "AcDbRadialDimension"),
        (15, 3.0), (25, 4.0), (35, 0.0),
        (40, 0.0),
    )
    diameter = entity(
        "DIMENSION", b"0",
        (100, "AcDbDimension"), (2, "*D2"),
        (10, 20.0), (20, 0.0), (30, 0.0),
        (11, 20.0), (21, 5.0), (31, 0.0),
        (70, 35),
        (1, b""),
        (42, 10.0),
        (3, "STANDARD"),
        (100, "AcDbDiametricDimension"),
        (15, 20.0), (25, 10.0), (35, 0.0),
        (40, 0.0),
    )
    angular = entity(
        "DIMENSION", b"0",
        (100, "AcDbDimension"), (2, "*D3"),
        (10, 44.33012701892219), (20, 2.5), (30, 0.0),
        (11, 46.0), (21, 3.0), (31, 0.0),
        (70, 37),
        (1, b""),
        (42, 1.0471975511965976),
        (3, "STANDARD"),
        (100, "AcDb3PointAngularDimension"),
        (13, 50.0), (23, 0.0), (33, 0.0),
        (14, 45.0), (24, 8.660254037844386), (34, 0.0),
        (15, 40.0), (25, 0.0), (35, 0.0),
    )
    ents = (circle(0, 0, 5) + circle(20, 5, 5) + arc(40, 0, 10, 0, 60)
            + radius + diameter + angular)
    return hdr + tbl + section("ENTITIES", ents) + pair(0, "EOF")


# --------------------------------------------------------------- fixture 13
def infinite_lines():
    """The two entities that have no end, in a drawing 0.002 units across.

    A RAY and an XLINE are mathematically infinite, so the renderer has to
    decide where to stop drawing them. Drawing them a fixed 1e6 units long
    (what uncad did before 0.3.0) is a coordinate, not a length: this
    drawing's crop is 0.002 units wide, so a 1568 px image is around 750
    thousand pixels per unit and that segment lands 7e11 px off the canvas,
    where tiny-skia's fixed-point scan converter asserts and takes the
    process with it. The right answer is the edge of the picture.

    | handle | entity | where | what it must look like |
    |---|---|---|---|
    | 30 | `LINE` | (0,0) to (0.002, 0.002) | the diagonal, and the entity the crop comes from |
    | 31 | `TEXT` "X" | (0.0013, 0.0002), height 0.0006 | one text class, so the export can build a tile pyramid at all |
    | 32 | `XLINE` | (0.001, 0.001), direction (1, 0) | a horizontal line across the whole image, through the middle |
    | 33 | `RAY` | (0.001, 0.001), direction (0, 1) | a vertical line from the middle to the *top* edge, with nothing below it |

    The base point of both construction lines is inside the LINE's box, so
    the crop of this drawing is the crop of its finite entities -- an
    infinite line must never enlarge it. The text sits in the bottom-right
    quarter, clear of the middle row and column, so a test can sample both
    construction lines (and the empty half below the ray) at pixels nothing
    else draws on.
    """
    hdr = header(INSUNITS=(70, 4))
    tbl = tables()
    ents = (
        line(0.0, 0.0, 0.002, 0.002, handle="30")
        + text("X", 0.0013, 0.0002, height=0.0006, handle="31")
        + construction("XLINE", 0.001, 0.001, 1.0, 0.0, handle="32")
        + construction("RAY", 0.001, 0.001, 0.0, 1.0, handle="33")
    )
    return hdr + tbl + section("ENTITIES", ents) + pair(0, "EOF")



# --------------------------------------------------------------- fixture 14
def polyline_vertices():
    """The three old-style POLYLINEs whose last vertex LibreDWG's own
    `dwg_object_polyline_{2,3}d_get_points` drops on every R13/R14/R2000
    file (its `first_vertex..last_vertex` loop ends before `last_vertex`).

    | handle | entity | vertices | what it pins |
    |---|---|---|---|
    | 30 | closed `POLYLINE_2D` | (0,0) (100,0) (100,100) (0,100) | a 100 x 100 square: area 10000, perimeter 400 -- a triangle if the 4th vertex is lost |
    | 35 | `POLYLINE_2D` | (0,1000) bulge 1.0, (100,1000) | one semicircular segment: length pi * 50, gone entirely if either the vertex or the bulge list is lost |
    | 39 | `POLYLINE_3D` | (0,0,0) (10,0,0) (10,10,0) (0,10,5) (0,0,5) | 5 vertices ending above the start, so the last one is the only vertex with both y = 0 and z = 5 |

    The VERTEX records name the POLYLINE in group 330, as AutoCAD writes
    them (the polyface fixture uses the other shape)."""
    hdr = header(INSUNITS=(70, 4))
    tbl = tables()
    square = polyline(
        "AcDb2dPolyline", "AcDb2dVertex",
        [(0, 0, 0), (100, 0, 0), (100, 100, 0), (0, 100, 0)],
        groups=[(70, 1)], vertex_flag=0,
        handle="30", owner="1F", vertex_handles=("31", "32", "33", "34"), seqend="3A",
    )
    arc = polyline(
        "AcDb2dPolyline", "AcDb2dVertex",
        [(0, 1000, 0, 1.0), (100, 1000, 0)],
        vertex_flag=0,
        handle="35", owner="1F", vertex_handles=("36", "37"), seqend="38",
    )
    p3d = polyline(
        "AcDb3dPolyline", "AcDb3dPolylineVertex",
        [(0, 0, 0), (10, 0, 0), (10, 10, 0), (0, 10, 5), (0, 0, 5)],
        groups=[(70, 8)], vertex_flag=32,
        handle="39", owner="1F",
        vertex_handles=("3B", "3C", "3D", "3E", "3F"), seqend="40",
    )
    return hdr + tbl + section("ENTITIES", square + arc + p3d) + pair(0, "EOF")


# --------------------------------------------------------------- fixture 15
def entity_truecolor():
    """The four ways a DXF entity can state its colour, on one layer whose
    own ACI is 3 (`00ff00`), so each entity's rendered colour says which
    field was read.

    | handle | groups | `color_index` | `true_color` |
    |---|---|---|---|
    | 30 | 420 only | 256 | `0x00ff7f` |
    | 31 | 62 only | 1 | none -- an index is not an RGB |
    | 32 | 62 then 420 | 1 | `0x0000ff` |
    | 33 | neither | 256 | none |

    65407 is `0x00ff7f` and 255 is `0x0000ff`; both are written as the plain
    24-bit decimal a DXF carries, with no method byte."""
    hdr = header(INSUNITS=(70, 4))
    tbl = tables(layers=((b"0", 7), (b"GREEN", 3)))
    ents = (
        line(0, 0, 100, 0, layer=b"GREEN", handle="30", owner="1F", true_color=65407)
        + line(0, 10, 100, 10, layer=b"GREEN", handle="31", owner="1F", color=1)
        + line(0, 20, 100, 20, layer=b"GREEN", handle="32", owner="1F", color=1,
               true_color=255)
        + line(0, 30, 100, 30, layer=b"GREEN", handle="33", owner="1F")
    )
    return hdr + tbl + section("ENTITIES", ents) + pair(0, "EOF")


# --------------------------------------------------------------- fixture 16
def polyface_mesh():
    """A polyface mesh and a polygon mesh next to an ordinary LINE, written
    the way ezdxf and several exporters do: the VERTEX records name the
    *block record* in group 330, not the POLYLINE.

    That shape is what made LibreDWG reject the whole file (a polygon mesh's
    `AcDbPolygonMeshVertex` marker matched no known subclass, which is a
    critical error) and what makes it type a polyface's vertices VERTEX_MESH
    rather than VERTEX_PFACE. The LINE is the control: it must survive.

    | handle | entity | what it pins |
    |---|---|---|
    | 30 | `LINE` (0,0)-(1000,0) | the rest of the file survives whatever the meshes do |
    | 31 | `POLYLINE_PFACE` | 8 vertices (two 10 x 10 squares, at z 0 and z 10), 2 quad faces -> 8 wireframe edges |
    | 50 | `POLYLINE_MESH` | an open 3 (M) by 4 (N) grid at (i*10, j*5, 0) -> 4*(3-1) + 3*(4-1) = 17 edges |

    Neither mesh's VERTEX records may appear in the entity list."""
    hdr = header(INSUNITS=(70, 4))
    tbl = tables()
    pface_vertices = [(0, 0, 0), (10, 0, 0), (10, 10, 0), (0, 10, 0),
                      (0, 0, 10), (10, 0, 10), (10, 10, 10), (0, 10, 10)]
    pface = polyline(
        "AcDbPolyFaceMesh", "AcDbPolyFaceMeshVertex", pface_vertices,
        groups=[(70, 64), (71, 8), (72, 2)], vertex_flag=192,
        handle="31", owner="1F", vertex_owner="1F",
        vertex_handles=("32", "33", "34", "35", "36", "37", "38", "39", "3A", "3B"),
        seqend="3C", faces=((1, 2, 3, 4), (5, 6, 7, 8)),
    )
    grid = [(i * 10, j * 5, 0) for i in range(3) for j in range(4)]
    mesh = polyline(
        "AcDbPolygonMesh", "AcDbPolygonMeshVertex", grid,
        groups=[(70, 16), (71, 3), (72, 4)], vertex_flag=64,
        handle="50", owner="1F", vertex_owner="1F",
        vertex_handles=tuple("%X" % h for h in range(0x51, 0x51 + len(grid))),
        seqend="60",
    )
    ents = line(0, 0, 1000, 0, handle="30", owner="1F") + pface + mesh
    return hdr + tbl + section("ENTITIES", ents) + pair(0, "EOF")


# --------------------------------------------------------------- fixture 17
def block_layer0():
    """AutoCAD's layer-0-in-a-block rule: geometry drawn on layer 0 inside a
    block definition is placed on the layer of the reference that inserts it.

    Block `SYM` is inserted once, on layer `RED` (ACI 1), with its own
    colour BYLAYER. Layer `BLUE` is ACI 5 and layer `0` is ACI 7 (black on
    a white page), so every child's rendered colour names the layer it
    resolved against.

    | handle | child | layer | colour | drawn |
    |---|---|---|---|---|
    | 42 | `LINE` (0,0)-(10,0) | 0 | BYLAYER | `#ff0000` -- the INSERT's layer, not layer 0's black |
    | 43 | `LINE` (0,2)-(10,2) | 0 | BYBLOCK | `#ff0000` -- the INSERT's own resolved colour |
    | 44 | `LINE` (0,4)-(10,4) | BLUE | BYLAYER | `#0000ff` -- a named layer inside a block is used as stored |
    | 45 | `TEXT` "L0" | 0 | BYLAYER | its text record's layer is `RED` |
    """
    hdr = header(INSUNITS=(70, 4))
    pre = tables(layers=((b"0", 7), (b"RED", 1), (b"BLUE", 5)),
                 block_records=(("1F", "*Model_Space"), ("40", "SYM")))
    body = (line(0, 0, 10, 0, handle="42", owner="40", color=256)
            + line(0, 2, 10, 2, handle="43", owner="40", color=0)
            + line(0, 4, 10, 4, layer=b"BLUE", handle="44", owner="40", color=256)
            + text("L0", 0, 6, height=2.5, handle="45", owner="40"))
    pre += section("BLOCKS", block("SYM", "40", ("41", "46"), body=body))
    ents = insert("SYM", 0, 0, "50", "1F", layer=b"RED")
    return hdr + pre + section("ENTITIES", ents) + pair(0, "EOF")


# --------------------------------------------------------------- fixture 18
def title_block():
    """A sheet that says what the drawing is -- the question a package of a
    real set is asked first -- with every string of it in paper space.

    Model space holds one LINE (0,0) -> (100,50) and no text at all, the
    shape of an AutoCAD sheet set: the title, the sheet number and the
    office name live on the paper, drawn once, while the model holds the
    geometry. A4 landscape (297 x 210 mm, 6.35 mm margins on every side, no
    plot origin, so LIMMIN/LIMMAX put the 297 x 210 sheet at
    (-6.35, -6.35) .. (290.65, 203.65), the printable corner at the origin)
    with:

    | handle | entity | where (paper units) | string |
    |---|---|---|---|
    | 25 | `TEXT` | (150, 20), height 8 | `GARDEN PAVILION` |
    | 26 | `INSERT` of `TITLEBLOCK` | (200, 10) | -- |
    | 42 | `TEXT` inside `TITLEBLOCK` | (10, 10) + the insert, height 5 | `SHEET 1 OF 2` |
    | 43 | `LINE` inside `TITLEBLOCK` | (0, 5) -> (90, 5) | the rule under it |
    | 2A | `VIEWPORT` | frame (150, 120) x (200, 120), VIEWCTR (50, 25), VIEWSIZE 60 | the model at 2:1 |

    The nested text's record id is `26/42`, and its anchor on the sheet is
    (200 + 10, 10 + 10) = (210, 20). The viewport's scale is 120 / 60 = 2
    paper units per model unit, and its model window is 100 x 60 model
    units centred on (50, 25), i.e. (0, -5) .. (100, 55)."""
    hdr = header(INSUNITS=(70, 4))
    pre = tables(block_records=(("1F", "*Model_Space"), ("1C", "*Paper_Space", "2B"),
                                ("40", "TITLEBLOCK")))
    title_body = (line(0, 5, 90, 5, handle="43", owner="40")
                  + text(b"SHEET 1 OF 2", 10, 10, height=5, handle="42", owner="40"))
    pre += section("BLOCKS",
                   block("*Model_Space", "1F", ("20", "21"))
                   + block("*Paper_Space", "1C", ("22", "23"), paper=True)
                   + block("TITLEBLOCK", "40", ("41", "44"), body=title_body))
    post = section("OBJECTS",
                   dictionary("C", "0", (("ACAD_LAYOUT", "1A"),))
                   + dictionary("1A", "C", (("Layout1", "2B"),))
                   + layout("2B", "1A", "Layout1", 1, block_record="1C", viewport="2A",
                            paper=("ISO_A4_(210.00_x_297.00_MM)", 210.0, 297.0)))
    vp = viewport("2A", "1C", (150.0, 120.0), (200.0, 120.0), (50.0, 25.0), 60.0)
    ents = (line(0, 0, 100, 50, handle="24", owner="1F")
            + text(b"GARDEN PAVILION", 150, 20, height=8, handle="25", owner="1C", paper=True)
            + insert("TITLEBLOCK", 200, 10, "26", "1C", paper=True)
            + vp)
    return hdr + pre + section("ENTITIES", ents) + post + pair(0, "EOF")


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
    if which in ("all", "plot-origin"):
        write("plot_origin_r2000.dxf", plot_origin())
    if which in ("all", "angular-ordinate"):
        write("angular_ordinate_r2000.dxf", angular_ordinate())
    if which in ("all", "radial"):
        write("radial_r2000.dxf", radial())
    if which in ("all", "viewport-states"):
        write("viewport_states_r2000.dxf", viewport_states())
    if which in ("all", "hatched-viewport"):
        write("hatched_viewport_r2000.dxf", hatched_viewport())
    if which in ("all", "nested-attrib"):
        write("nested_attrib_r2000.dxf", nested_attrib())
    if which in ("all", "infinite-lines"):
        write("infinite_lines_r2000.dxf", infinite_lines())
    if which in ("all", "polyline-vertices"):
        write("polyline_vertices_r2000.dxf", polyline_vertices())
    if which in ("all", "entity-truecolor"):
        write("entity_truecolor_r2000.dxf", entity_truecolor())
    if which in ("all", "polyface-mesh"):
        write("polyface_mesh_r2000.dxf", polyface_mesh())
    if which in ("all", "block-layer0"):
        write("block_layer0_r2000.dxf", block_layer0())
    if which in ("all", "title-block"):
        write("title_block_r2000.dxf", title_block())
    if which == "dimlfac-minimal":
        write("dimlfac12_r2000.dxf", dimlfac12("minimal"))
    if which == "viewport-minimal":
        write("twisted_viewport_r2000.dxf", twisted_viewport("minimal"))
