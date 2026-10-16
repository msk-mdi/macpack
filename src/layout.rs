//! The Control Center layout (macOS 26+), stored per host in
//! `com.apple.controlcenter.bentoboxes` as JSON describing a 4-column grid.
//! Each grid element points by UUID at a module listed in an embedded,
//! base64-encoded binary plist.
//!
//! In profiles the layout is plain text, one tile per `;`-separated entry:
//! `WiFi 2x1 at 0,0; NowPlaying 2x2 at 2,0; Hearing 1x1 at 2,2`. A tile ends
//! with `as <size>` when its module size isn't the one implied by its area.

use std::collections::HashMap;

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use plist::{Dictionary, Value as Plist};
use serde_json::{Value as Json, json};

/// Grid width in cells.
const WIDTH: u32 = 4;
/// Rows macOS reserves even for a short layout.
const MIN_HEIGHT: u32 = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// Module id, e.g. `WiFi`, `Sound`, `NowPlaying`.
    pub module: String,
    pub w: u32,
    pub h: u32,
    pub x: u32,
    pub y: u32,
    /// Module size: `small`, `medium` or `large`.
    pub size: String,
}

/// The module size macOS uses for a tile of this area.
fn implied_size(w: u32, h: u32) -> &'static str {
    match w * h {
        1 => "small",
        2 => "medium",
        _ => "large",
    }
}

/// Parses layout text and returns it in canonical form.
pub fn normalize(text: &str) -> Result<String> {
    Ok(format(&parse(text)?))
}

/// Parses layout text, checking that tiles fit the grid and don't overlap.
pub fn parse(text: &str) -> Result<Vec<Tile>> {
    let mut tiles = Vec::new();
    for item in text.split([';', '\n']).map(str::trim).filter(|s| !s.is_empty()) {
        tiles.push(parse_tile(item).with_context(|| format!("bad tile {item:?}"))?);
    }
    check(&mut tiles)?;
    Ok(tiles)
}

fn parse_tile(item: &str) -> Result<Tile> {
    let words: Vec<&str> = item.split_whitespace().collect();
    let (module, dims, pos, size) = match words[..] {
        [m, d, "at", p] => (m, d, p, None),
        [m, d, "at", p, "as", s] => (m, d, p, Some(s)),
        _ => bail!("expected `<Module> <W>x<H> at <X>,<Y>`"),
    };
    ensure!(module.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "bad module name");
    let (w, h) = pair(dims, 'x')?;
    let (x, y) = pair(pos, ',')?;
    ensure!((1..=WIDTH).contains(&w) && (1..=WIDTH).contains(&h), "size must be 1 to {WIDTH} cells each way");
    let size = size.unwrap_or(implied_size(w, h));
    ensure!(["small", "medium", "large"].contains(&size), "size must be small, medium or large");
    Ok(Tile { module: module.to_string(), w, h, x, y, size: size.to_string() })
}

fn pair(s: &str, sep: char) -> Result<(u32, u32)> {
    let (a, b) = s.split_once(sep).with_context(|| format!("expected two numbers in {s:?}"))?;
    Ok((a.parse()?, b.parse()?))
}

/// Sorts tiles into reading order and rejects overlaps or tiles off the grid.
fn check(tiles: &mut [Tile]) -> Result<()> {
    tiles.sort_by_key(|t| (t.y, t.x));
    let mut taken: HashMap<(u32, u32), &str> = HashMap::new();
    for t in tiles.iter() {
        ensure!(t.x + t.w <= WIDTH, "{} at {},{} runs past the {WIDTH}-column grid", t.module, t.x, t.y);
        for cell in cells(t) {
            if let Some(other) = taken.insert(cell, &t.module) {
                bail!("{} overlaps {other} at {},{}", t.module, cell.0, cell.1);
            }
        }
    }
    Ok(())
}

fn cells(t: &Tile) -> impl Iterator<Item = (u32, u32)> + '_ {
    (t.y..t.y + t.h).flat_map(move |y| (t.x..t.x + t.w).map(move |x| (x, y)))
}

pub fn format(tiles: &[Tile]) -> String {
    let items: Vec<String> = tiles
        .iter()
        .map(|t| {
            let mut s = format!("{} {}x{} at {},{}", t.module, t.w, t.h, t.x, t.y);
            if t.size != implied_size(t.w, t.h) {
                s += &format!(" as {}", t.size);
            }
            s
        })
        .collect();
    items.join("; ")
}

/// Reads tiles from the `boxes` JSON stored by Control Center.
pub fn decode(boxes: &[u8]) -> Result<Vec<Tile>> {
    let json: Json = serde_json::from_slice(boxes).context("layout is not JSON")?;
    let boxes = json.as_array().context("layout is not a list")?;
    let [b] = &boxes[..] else { bail!("expected one Control Center page, found {}", boxes.len()) };

    let data = b["displayablesData"].as_str().context("missing displayablesData")?;
    let displayables = Plist::from_reader(std::io::Cursor::new(BASE64.decode(data)?))?;
    let modules = displayables
        .as_dictionary()
        .and_then(|d| d.get("modules"))
        .and_then(Plist::as_data)
        .context("missing module list")?;
    let modules = Plist::from_reader(std::io::Cursor::new(modules))?;
    let mut by_id: HashMap<&str, (&str, &str)> = HashMap::new();
    for m in modules.as_array().context("module list is not a list")? {
        let m = m.as_dictionary().context("module is not a dictionary")?;
        let id = m.get("id").and_then(raw_value).and_then(Plist::as_string);
        let module = m.get("moduleId").and_then(Plist::as_string);
        let size = m.get("size").and_then(Plist::as_dictionary).and_then(|d| d.keys().next());
        if let (Some(id), Some(module), Some(size)) = (id, module, size) {
            by_id.insert(id, (module, size));
        }
    }

    let mut tiles = Vec::new();
    let mut foreign = 0;
    for e in b["grid"]["elements"].as_array().context("missing grid elements")? {
        let id = e["id"]["rawValue"].as_str().context("grid element without id")?;
        let Some(&(module, size)) = by_id.get(id) else {
            foreign += 1;
            continue;
        };
        let shape = e["size"].as_object().and_then(|o| o.keys().next()).context("grid element without size")?;
        let (w, h) = shape
            .strip_prefix("square")
            .or_else(|| shape.strip_prefix("rect"))
            .and_then(|d| pair(d, 'x').ok())
            .with_context(|| format!("unknown tile size {shape:?}"))?;
        let coord = |k: &str| e["location"][k].as_u64().map(|n| n as u32).context("grid element without location");
        tiles.push(Tile { module: module.to_string(), w, h, x: coord("x")?, y: coord("y")?, size: size.to_string() });
    }
    if foreign > 0 {
        bail!("contains {foreign} control(s) from other apps, which macpack can't transfer");
    }
    check(&mut tiles)?;
    Ok(tiles)
}

fn raw_value(v: &Plist) -> Option<&Plist> {
    v.as_dictionary()?.get("rawValue")
}

/// Builds the `boxes` JSON for `tiles`, with fresh instance ids.
pub fn encode(tiles: &[Tile]) -> Result<Vec<u8>> {
    let mut elements = Vec::new();
    let mut modules = Vec::new();
    let mut taken = Vec::new();
    for t in tiles {
        let id = uuid::Uuid::new_v4().to_string().to_uppercase();
        let shape = if t.w == t.h { format!("square{}x{}", t.w, t.h) } else { format!("rect{}x{}", t.w, t.h) };
        elements.push(json!({ "id": { "rawValue": id }, "size": { shape: {} }, "location": { "x": t.x, "y": t.y } }));
        modules.push(Plist::Dictionary(Dictionary::from_iter([
            ("id".to_string(), dict([("rawValue", Plist::String(id))])),
            ("size".to_string(), dict([(t.size.as_str(), Plist::Dictionary(Dictionary::new()))])),
            ("moduleId".to_string(), Plist::String(t.module.clone())),
        ])));
        taken.extend(cells(t));
    }

    let instance_ids: Vec<Plist> = modules
        .iter()
        .filter_map(|m| m.as_dictionary().and_then(|d| d.get("id")).cloned())
        .collect();
    let displayables = dict([
        ("instanceIds", Plist::Array(instance_ids)),
        ("modules", Plist::Data(binary(&Plist::Array(modules))?)),
    ]);

    let height = tiles.iter().map(|t| t.y + t.h + 1).max().unwrap_or(0).max(MIN_HEIGHT);
    let all: Vec<(u32, u32)> = (0..height).flat_map(|y| (0..WIDTH).map(move |x| (x, y))).collect();
    let locations = |keep: &dyn Fn(&(u32, u32)) -> bool| -> Vec<Json> {
        all.iter().filter(|c| keep(c)).map(|(x, y)| json!({ "x": x, "y": y })).collect()
    };
    let boxes = json!([{
        "menuBarIconName": "circle.grid.2x2",
        "displayablesData": BASE64.encode(binary(&displayables)?),
        "grid": {
            "id": { "rawValue": 0 },
            "width": WIDTH,
            "maxHeight": height,
            "elements": elements,
            "allLocations": locations(&|_| true),
            "occupiedLocations": locations(&|c| taken.contains(c)),
            "unoccupiedLocations": locations(&|c| !taken.contains(c)),
        },
    }]);
    Ok(serde_json::to_vec(&boxes)?)
}

fn dict<'a>(entries: impl IntoIterator<Item = (&'a str, Plist)>) -> Plist {
    Plist::Dictionary(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn binary(value: &Plist) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    value.to_writer_binary(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layout saved by macOS 27, including stale module entries that no
    /// grid element points at.
    const SAMPLE: &[u8] = include_bytes!("testdata/controlcenter-boxes.json");
    const SAMPLE_TEXT: &str = "WiFi 2x1 at 0,0; NowPlaying 2x2 at 2,0; Bluetooth 2x1 at 0,1; AirDrop 2x1 at 0,2; \
        Hearing 1x1 at 2,2; ScreenMirroring 1x1 at 3,2; KeyboardBrightness 2x1 at 0,3; FocusModes 2x1 at 2,3; \
        Display 4x1 at 0,4; Sound 4x1 at 0,5; MusicRecognition 2x1 at 2,6";

    #[test]
    fn decodes_a_real_layout() {
        assert_eq!(format(&decode(SAMPLE).unwrap()), SAMPLE_TEXT);
    }

    #[test]
    fn round_trips_through_encode() {
        let tiles = parse(SAMPLE_TEXT).unwrap();
        assert_eq!(decode(&encode(&tiles).unwrap()).unwrap(), tiles);

        let odd = parse("Sound 2x1 at 0,0 as large").unwrap();
        assert_eq!(decode(&encode(&odd).unwrap()).unwrap(), odd);
        assert_eq!(format(&odd), "Sound 2x1 at 0,0 as large");
    }

    #[test]
    fn encodes_grid_bookkeeping() {
        let boxes: Json = serde_json::from_slice(&encode(&parse("WiFi 2x1 at 0,0").unwrap()).unwrap()).unwrap();
        let grid = &boxes[0]["grid"];
        assert_eq!(grid["maxHeight"], 8);
        assert_eq!(grid["allLocations"].as_array().unwrap().len(), 32);
        assert_eq!(grid["occupiedLocations"], json!([{ "x": 0, "y": 0 }, { "x": 1, "y": 0 }]));
        assert_eq!(grid["elements"][0]["size"], json!({ "rect2x1": {} }));
    }

    #[test]
    fn normalizes_text() {
        let text = "\n  Sound 4x1 at 0,1\n WiFi 2x1 at 0,0 ;; ";
        assert_eq!(normalize(text).unwrap(), "WiFi 2x1 at 0,0; Sound 4x1 at 0,1");
        assert_eq!(normalize("").unwrap(), "");
    }

    #[test]
    fn rejects_bad_layouts() {
        for bad in [
            "WiFi 2x1 at 3,0",                   // off the grid
            "WiFi 2x1 at 0,0; Sound 1x1 at 1,0", // overlap
            "WiFi 2x1 0,0",
            "WiFi 0x1 at 0,0",
            "WiFi 2x1 at 0,0 as huge",
            "Wi;Fi 2x1 at 0,0",
            "Wi\"Fi 2x1 at 0,0",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn refuses_controls_from_other_apps() {
        let mut boxes: Json = serde_json::from_slice(&encode(&parse("WiFi 2x1 at 0,0").unwrap()).unwrap()).unwrap();
        let elements = boxes[0]["grid"]["elements"].as_array_mut().unwrap();
        elements.push(json!({ "id": { "rawValue": "OTHER" }, "size": { "square1x1": {} }, "location": { "x": 3, "y": 3 } }));
        let err = decode(&serde_json::to_vec(&boxes).unwrap()).unwrap_err().to_string();
        assert!(err.contains("other apps"), "{err}");
    }
}
