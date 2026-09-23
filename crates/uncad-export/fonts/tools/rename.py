import sys
from fontTools.ttLib import TTFont
src, dst, family, psname = sys.argv[1:5]
f = TTFont(src)
full = family + " Regular"
for rec in f['name'].names:
    if rec.nameID in (1, 16): rec.string = family
    elif rec.nameID == 3: rec.string = f"{psname};uncad-subset"
    elif rec.nameID == 4: rec.string = full
    elif rec.nameID == 6: rec.string = psname
    elif rec.nameID == 5: rec.string = rec.toUnicode() + "; uncad subset"
if 'CFF ' in f:
    cff = f['CFF '].cff
    cff.fontNames[0] = psname
    td = cff.topDictIndex[0]
    td.FamilyName = family; td.FullName = full
f.save(dst)
g = TTFont(dst); n = g['name']
print("renamed ->", dst, "| family:", n.getDebugName(1), "| full:", n.getDebugName(4), "| ps:", n.getDebugName(6), "| id3:", n.getDebugName(3), "| ver:", n.getDebugName(5))
if 'CFF ' in g: print("CFF fontName:", g['CFF '].cff.fontNames[0], "FamilyName:", g['CFF '].cff.topDictIndex[0].FamilyName)
