//! Writing the package: files and their listing, record shards and their
//! index, the 8-bit RGB images, the tile hashes, and clearing what a
//! previous package in the same directory left.

use std::path::Path;

use serde::Serialize;
use serde_json::{json, Value};

use super::records::{id_key, Record};
use super::{ExportError, SCHEMA};

/// One file the package holds.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WrittenFile {
    /// Relative to the package directory, `/`-separated.
    pub path: String,
    /// `None` for the three files written after the listing was made
    /// (`manifest.json`, `README.txt`, `report.json`).
    pub bytes: Option<u64>,
    pub kind: String,
}

pub(crate) struct Writer<'a> {
    pub(crate) dir: &'a Path,
    pub(crate) files: Vec<WrittenFile>,
    pub(crate) shard_index: Vec<Value>,
    pub(crate) units: Value,
    pub(crate) shard_kb: usize,
}

impl Writer<'_> {
    pub(crate) fn write_bytes(
        &mut self,
        rel: &str,
        bytes: &[u8],
        kind: &str,
    ) -> Result<(), ExportError> {
        let path = self.dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ExportError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        std::fs::write(&path, bytes).map_err(|source| ExportError::Io { path, source })?;
        self.files.push(WrittenFile {
            path: rel.to_string(),
            bytes: Some(bytes.len() as u64),
            kind: kind.to_string(),
        });
        Ok(())
    }

    pub(crate) fn write_json(
        &mut self,
        rel: &str,
        value: &Value,
        kind: &str,
    ) -> Result<(), ExportError> {
        let text = serde_json::to_string_pretty(value)?;
        self.write_bytes(rel, text.as_bytes(), kind)
    }

    /// [`Writer::write_json`] without the indentation, for a file whose own
    /// size is the thing being kept: a sidecar drops record rows until its
    /// compact form fits its cap, so pretty-printing it afterwards would put
    /// a file three times the measured size on disk. Record shards are
    /// written compact for the same reason.
    pub(crate) fn write_json_compact(
        &mut self,
        rel: &str,
        value: &Value,
        kind: &str,
    ) -> Result<(), ExportError> {
        let text = serde_json::to_string(value)?;
        self.write_bytes(rel, text.as_bytes(), kind)
    }

    /// Writes `records` (already in id order) as `name.json`, or as
    /// `name.NNN.json` shards above `shard_kb`, and indexes them.
    pub(crate) fn write_records(
        &mut self,
        name: &str,
        kind: &str,
        records: &[Record],
    ) -> Result<(), ExportError> {
        let limit = self.shard_kb.max(1) * 1024;
        let mut shards: Vec<Vec<&Record>> = vec![Vec::new()];
        let mut bytes = 0usize;
        for r in records {
            let size = serde_json::to_string(&r.value)?.len() + 8;
            if bytes + size > limit && !shards.last().is_none_or(Vec::is_empty) {
                shards.push(Vec::new());
                bytes = 0;
            }
            shards.last_mut().expect("one shard").push(r);
            bytes += size;
        }
        let single = shards.len() == 1;
        for (n, shard) in shards.iter().enumerate() {
            let file = if single {
                format!("{name}.json")
            } else {
                format!("{name}.{:03}.json", n + 1)
            };
            let value = json!({
                "$schema": SCHEMA,
                "kind": kind,
                "units": self.units,
                "count": shard.len(),
                "records": shard.iter().map(|r| Value::Object(r.value.clone())).collect::<Vec<_>>(),
            });
            let text = serde_json::to_string(&value)?;
            self.write_bytes(&file, text.as_bytes(), kind)?;
            // `first_id`/`last_id` alone are no index: ids are paths of
            // numbers written as strings, ordered by their numbers, so the
            // string comparison a consumer would reach for picks the wrong
            // shard ("10" sorts before "9"). `first_key`/`last_key` publish
            // the order itself: the id's first number.
            let key = |r: Option<&&Record>| r.map(|r| id_key(&r.id)[0]);
            self.shard_index.push(json!({
                "file": file,
                "kind": kind,
                "first_id": shard.first().map(|r| r.id.as_str()),
                "last_id": shard.last().map(|r| r.id.as_str()),
                "first_key": key(shard.first()),
                "last_key": key(shard.last()),
                "count": shard.len(),
                "bytes": text.len(),
            }));
        }
        Ok(())
    }
}

/// Removes what a previous uncad package in `dir` left behind, so a second
/// export with other options does not leave stale shards, tile PNGs and
/// sidecars beside the new ones: they look valid (same schema, same record
/// ids) and a consumer that walks the tree would mix two exports.
///
/// Only the files the previous `manifest.json` lists are removed, and only
/// when it is an uncad manifest: a directory holding anything else is left
/// alone, and a listed path that is not a plain relative path inside `dir`
/// is ignored. Directories under `frames/` and `sheets/` go when they are
/// left empty. Every failure is ignored -- the write that follows reports
/// what actually matters.
pub(crate) fn clear_previous_package(dir: &Path) {
    let Ok(text) = std::fs::read_to_string(dir.join("manifest.json")) else {
        return;
    };
    let Ok(manifest) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let ours = manifest
        .get("$schema")
        .and_then(Value::as_str)
        .is_some_and(|s| s.starts_with("uncad-package/"));
    if !ours {
        return;
    }
    let files = manifest.get("files").and_then(Value::as_array);
    for file in files.into_iter().flatten() {
        let Some(rel) = file.get("path").and_then(Value::as_str) else {
            continue;
        };
        let inside = !rel.is_empty()
            && Path::new(rel)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)));
        if inside {
            let _ = std::fs::remove_file(dir.join(rel));
        }
    }
    for sub in ["frames", "sheets"] {
        remove_empty_dirs(&dir.join(sub));
    }
}

/// Removes `dir` and every directory under it that is empty once its own
/// empty children are gone; a directory still holding a file stays (with
/// everything above it).
fn remove_empty_dirs(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            remove_empty_dirs(&entry.path());
        }
    }
    // Fails, harmlessly, when anything is left in it.
    let _ = std::fs::remove_dir(dir);
}

/// The renderer's PNG (RGBA, on an opaque white background) as 8-bit RGB:
/// a quarter fewer bytes to decode for an image that has no transparency
/// to carry, and the form the package has always written.
pub(crate) fn to_rgb(rgba_png: &[u8]) -> Result<Vec<u8>, ExportError> {
    let encode = |e: &dyn std::fmt::Display| ExportError::Encode(e.to_string());
    let mut decoder = png::Decoder::new(std::io::Cursor::new(rgba_png));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| encode(&e))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| ExportError::Encode("the image is too large to decode".into()))?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(|e| encode(&e))?;
    let samples = info.color_type.samples();
    let data = &buf[..info.buffer_size()];
    let rgb: Vec<u8> = match samples {
        4 => data
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|px| {
                // Straight colour over white: the renderer fills the
                // background, so alpha is 255 and this is the identity.
                let a = u32::from(px[3]);
                let over = |c: u8| ((u32::from(c) * a + 255 * (255 - a) + 127) / 255) as u8;
                [over(px[0]), over(px[1]), over(px[2])]
            })
            .collect(),
        3 => data.to_vec(),
        _ => {
            return Err(ExportError::Encode(format!(
                "unexpected PNG colour type {:?}",
                info.color_type
            )))
        }
    };
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, info.width, info.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| encode(&e))?;
        writer.write_image_data(&rgb).map_err(|e| encode(&e))?;
        writer.finish().map_err(|e| encode(&e))?;
    }
    Ok(out)
}

/// SHA-256 of `data` as lower-case hex (FIPS 180-4), for the per-tile
/// hashes `tiles.json` publishes: a package verifier can check a tile PNG
/// against the manifest without re-running the export. Written out here
/// rather than pulled in: 40 lines of a fully specified function, pinned by
/// the standard test vectors below.
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    let (blocks, _) = padded.as_chunks::<64>();
    for block in blocks {
        let mut w = [0u32; 64];
        let (words, _) = block.as_chunks::<4>();
        for (word, bytes) in w.iter_mut().zip(words) {
            *word = u32::from_be_bytes(*bytes);
        }
        for i in 16..64 {
            let (a, b) = (w[i - 15], w[i - 2]);
            let s0 = a.rotate_right(7) ^ a.rotate_right(18) ^ (a >> 3);
            let s1 = b.rotate_right(17) ^ b.rotate_right(19) ^ (b >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for (k, wi) in K.iter().zip(w.iter()) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(*k)
                .wrapping_add(*wi);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_published_vectors() {
        // FIPS 180-4's own examples, plus the one-block/two-block boundary
        // (55, 56 and 64 bytes, where the padding runs into a second
        // block).
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            sha256_hex(&[b'a'; 1000]),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
        for len in [55, 56, 63, 64, 65] {
            let hash = sha256_hex(&vec![0u8; len]);
            assert_eq!(hash.len(), 64, "{len} bytes");
            assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        }
        assert_eq!(
            sha256_hex(&[b'a'; 56]),
            "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
        );
    }

    #[test]
    fn an_rgba_png_on_white_becomes_the_same_picture_in_rgb() {
        // A 2 x 1 RGBA image: one opaque red pixel, one half-transparent
        // black one, which over white is mid-grey.
        let mut rgba = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut rgba, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[255, 0, 0, 255, 0, 0, 0, 128])
                .unwrap();
        }
        let rgb = to_rgb(&rgba).expect("re-encodes");
        let decoder = png::Decoder::new(std::io::Cursor::new(&rgb));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgb);
        assert_eq!(&buf[..6], &[255, 0, 0, 127, 127, 127]);
    }
}
