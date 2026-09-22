# Golden cases

Copies of what the `uncad-model` repository's golden writer produces, one pair per case:

| File | What it is |
|---|---|
| `g1.dxf` | The synthetic drawing (R2000 ASCII DXF) written from the G1 spec |
| `g1.expected.json` | The model that spec says a reader must produce from it |

`tests/golden.rs` parses the DXF and requires the model to match exactly.

The pair is generated, not hand-written. To regenerate after a change to the writer or the
spec, from a checkout of `uncad-model`:

```
cargo run -p uncad-model-golden --example write_case -- g1 g1.dxf g1.expected.json
```

and copy both files here. A tree that carries both repositories side by side checks that
the copies have not drifted from the writer.
