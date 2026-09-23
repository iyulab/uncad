# Golden cases

Copies of what the `uncad-model` repository's golden writer produces, one pair per case:

| File | What it is |
|---|---|
| `<case>.dxf` | The synthetic drawing (R2000 ASCII DXF) written from that case's spec |
| `<case>.expected.json` | The model that spec says a reader must produce from it |

Cases here: `g1`, `g2`, `g5`, `g6`, `g7`, `g8`, `g9`, `g10`, `g15`. `tests/golden.rs` parses
each DXF and requires the model to match exactly, apart from the known deviations it
applies to the expectation (each with a test that fails the day it is no longer needed).

`g8.dxf` is not UTF-8 by design: it declares `$DWGCODEPAGE = ANSI_949` and stores its
Korean text as CP949 bytes, which is how an R2000 DXF carries such text. An editor shows
those strings as mojibake; the expected JSON has them as UTF-8. Do not "fix" the encoding of
the file.

The pair is generated, not hand-written. To regenerate after a change to the writer or the
spec, from a checkout of `uncad-model`:

```
cargo run -p uncad-model-golden --example write_case -- <case> <case>.dxf <case>.expected.json
```

and copy both files here. A tree that carries both repositories side by side checks that
the copies have not drifted from the writer.
