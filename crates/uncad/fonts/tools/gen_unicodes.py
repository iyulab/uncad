ks = [cp for cp in range(0xAC00, 0xD7A4) if len(chr(cp).encode('euc_kr')) == 2]
assert len(ks) == 2350, len(ks)
lines = [
 "# Basic Latin", "U+0020-007E",
 "# Latin-1 Supplement (degree B0, plus-minus B1, sup2 B2, sup3 B3, times D7, Oslash D8, oslash F8)", "U+00A0-00FF",
 "# Latin Extended-A", "U+0100-017F",
 "# Greek and Coptic (phi, delta, omega, ...)", "U+0370-03FF",
 "# General Punctuation (primes, dashes, bullets)", "U+2000-206F",
 "# Hangul Compatibility Jamo", "U+3130-318F",
 "# CAD symbols: empty set (diameter), diameter sign, arrows, ㎡ ㎜ ㎥ ㎝ ㎞", "U+2205 U+2300 U+2190-2193 U+33A1 U+339C U+33A5 U+339D U+339E",
 "# KS X 1001 Hangul syllables (2350)",
]
lines += ["U+%04X" % cp for cp in ks]
open('subset/unicodes.txt', 'w', encoding='utf-8').write("\n".join(lines) + "\n")
print("wrote subset/unicodes.txt with", len(ks), "syllables")
