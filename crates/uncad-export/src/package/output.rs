//! Writing the package: files and their listing, record shards and their
//! index, the 8-bit RGB images, and clearing what a previous package in the
//! same directory left.

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

#[cfg(test)]
mod tests {
    use super::*;

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
