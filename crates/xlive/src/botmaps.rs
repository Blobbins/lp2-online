//! Bots need a map's AI navigation data, which only the Training ("AI Enemy Battle") versions of
//! the versus maps ship. Training stage aArea_TUTOREAL_NN is an online map plus three script-list
//! entries (Tool\ScrList, an XFS file): mType 0x1d `tool\AI\NaviMesh\..\List_..` (navigation mesh
//! list), 0x1e `..\Way_..` (way points), 0x1f `tool\AI\AreaAI\aArea_TUTOREAL_NN` (area AI).
//!
//! `prepare` copies those resources into the online stage archive and puts the three entries into
//! empty slots of the online script list. Maps without a Training version get them generated from
//! their walking collision instead (lp2_arc::navgen; the area AI has no zones). The original
//! archive is kept as `<name>.arc.orig`; an archive that already has one is left alone.

use crate::log;
use std::path::Path;

/// Online map -> its Training version. ONLINE_12 ("Cube") has two layouts; 26 is layout 0.
const PAIRS: [(&str, &str); 4] = [("12", "26"), ("10", "31"), ("01", "41"), ("09", "46")];
const SCRLIST: u32 = 0x2E59_0330;
const AI_TYPES: [u32; 3] = [0x1D, 0x1E, 0x1F];
/// An empty script-list entry: no type, empty name, four zero parameters.
const EMPTY_LEN: usize = 0x35;

/// (offset, length, mType) of each entry of a script list, and the root object's offset.
fn records(d: &[u8]) -> Option<(usize, Vec<(usize, usize, u32)>)> {
    let u16_at = |o: usize| Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?));
    let u32_at = |o: usize| Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?));
    let root = 0x10 + u32_at(0xC)? as usize; // after the class/field definitions
    // Sizes count from the size field: root = 01 00 00 00, size, mAutoDelete, mpArray count, entries.
    if root + 4 + u32_at(root + 4)? as usize != d.len() {
        return None;
    }
    let mut o = root + 8 + 5 + 4;
    let mut out = vec![];
    while o < d.len() {
        if u16_at(o)? != 3 {
            return None;
        }
        let len = 4 + u32_at(o + 4)? as usize;
        out.push((o, len, u32_at(o + 12)?));
        o += len;
    }
    (o == d.len()).then_some((root, out))
}

/// A script-list entry naming a resource, laid out like the Training lists' AI entries: object
/// header (its index is set on insert), mType, name, four zero parameters.
pub fn ai_record(mtype: u32, name: &str) -> Vec<u8> {
    let mut f = [1u32, mtype, 1].map(u32::to_le_bytes).concat();
    f.extend(name.as_bytes());
    f.push(0);
    f.extend([1u32, 0, 1, 0, 1, 0, 1, 0].map(u32::to_le_bytes).concat());
    [&3u16.to_le_bytes()[..], &[0, 0], &(f.len() as u32 + 4).to_le_bytes(), &f].concat()
}

/// A script list's AI entries.
fn ai_records(list: &[u8]) -> Option<Vec<Vec<u8>>> {
    let (_, recs) = records(list)?;
    Some(recs.iter().filter(|r| AI_TYPES.contains(&r.2)).map(|&(o, n, _)| list[o..o + n].to_vec()).collect())
}

/// The online script list with the three AI entries `ai` in its first empty slots.
pub fn add_ai_entries(online: &[u8], ai: &[Vec<u8>]) -> Option<Vec<u8>> {
    let (root, ons) = records(online)?;
    let empty: Vec<_> = ons.iter().filter(|r| r.1 == EMPTY_LEN && r.2 == 0).take(3).collect();
    if ai.len() != 3 || empty.len() != 3 {
        return None;
    }
    let mut out = online.to_vec();
    // Back to front so earlier offsets stay valid; each entry keeps the slot's object index.
    for (&&(o, n, _), rec) in empty.iter().zip(ai).rev() {
        let mut rec = rec.clone();
        rec[2..4].copy_from_slice(&online[o + 2..o + 4]);
        out.splice(o..o + n, rec);
    }
    let size = (out.len() - root - 4) as u32;
    out[root + 4..root + 8].copy_from_slice(&size.to_le_bytes());
    records(&out).map(|_| out)
}

fn read_arc(p: &Path) -> Result<Vec<lp2_arc::Entry>, String> {
    std::fs::read(p).map_err(|e| format!("{}: {e}", p.display())).and_then(|b| lp2_arc::read(&b).map_err(|e| e.to_string()))
}

fn script_list<'a>(es: &'a [lp2_arc::Entry], stage: &str) -> Option<usize> {
    es.iter().position(|e| e.type_hash == SCRLIST && e.name.eq_ignore_ascii_case(&format!(r"Tool\ScrList\{stage}")))
}

/// The AI script-list entries and resources for online map `online`: its Training version's, or
/// generated from its walking collision.
fn ai_data(dir: &Path, online: &str) -> Result<(Vec<Vec<u8>>, Vec<lp2_arc::Entry>), String> {
    if let Some((_, training)) = PAIRS.iter().find(|p| p.0 == online) {
        let tr = read_arc(&dir.join(format!("aArea_TUTOREAL_{training}.arc")))?;
        let list = script_list(&tr, &format!("aArea_TUTOREAL_{training}")).ok_or("no training script list")?;
        let recs = ai_records(&tr[list].data).ok_or("training script list layout not as expected")?;
        let ai = |e: &&lp2_arc::Entry| {
            let n = e.name.to_ascii_lowercase();
            n.starts_with(r"tool\ai\areaai\") || n.starts_with(r"tool\ai\navimesh\")
        };
        return Ok((recs, tr.iter().filter(ai).cloned().collect()));
    }
    use lp2_arc::navgen;
    let v = read_arc(&dir.join(format!(r"variable\aArea_ONLINE_{online}_v.arc")))?;
    let c = lp2_arc::sbc::parse(&navgen::walking_collision(&v).ok_or("no walking collision")?.data)?;
    let folder = format!("Online_{online}");
    let t = std::time::Instant::now();
    let (list, way, ibl) = navgen::generate(&c, &folder, &navgen::PARAMS).map_err(|e| e.to_string())?;
    log!(
        "bots: aArea_ONLINE_{online}: generated {} navigation triangles and {} way points in {} ms",
        list.entries[0].mesh.polygons.len(),
        way.nodes.len(),
        t.elapsed().as_millis()
    );
    let files = navgen::entries(&format!("aArea_ONLINE_{online}"), &folder, &list, &way, &ibl).map_err(|e| e.to_string())?;
    Ok((files.iter().zip(AI_TYPES).map(|(e, t)| ai_record(t, &e.name)).collect(), files.into()))
}

fn prepare_one(dir: &Path, online: &str) -> Result<bool, String> {
    let arc = dir.join(format!("aArea_ONLINE_{online}.arc"));
    let orig = dir.join(format!("aArea_ONLINE_{online}.arc.orig"));
    if orig.exists() {
        return Ok(false);
    }
    let mut entries = read_arc(&arc)?;
    let on_i = script_list(&entries, &format!("aArea_ONLINE_{online}")).ok_or("no online script list")?;
    let (recs, files) = ai_data(dir, online)?;
    entries[on_i].data = add_ai_entries(&entries[on_i].data, &recs).ok_or("script list layout not as expected")?;
    entries.extend(files);
    let packed = lp2_arc::write(&entries).map_err(|e| e.to_string())?;
    std::fs::copy(&arc, &orig).map_err(|e| e.to_string())?;
    std::fs::write(&arc, packed).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Message tables (`message\msgPC\Msg*`, "MES\0"): entries are a u32 length, the UTF-8 text and a NUL.
const MES: u32 = 0x3FDA_9B90;
const WAITING: &str = "Waiting for other players to join...";
const WAITING_HINT: &str = "Waiting for other players to join...      Host: F7 add bot, F8 remove bot";

fn mes_entry(text: &str) -> Vec<u8> {
    let mut e = (text.len() as u32).to_le_bytes().to_vec();
    e.extend(text.as_bytes());
    e.push(0);
    e
}

/// `table` with its one entry `from` replaced by `to`.
pub fn replace_message(table: &[u8], from: &str, to: &str) -> Option<Vec<u8>> {
    let (old, new) = (mes_entry(from), mes_entry(to));
    let at = table.windows(old.len()).position(|w| w == old)?;
    table[at + 1..].windows(old.len()).all(|w| w != old).then(|| [&table[..at], &new[..], &table[at + old.len()..]].concat())
}

/// Puts the bot hotkeys into the lobby's "Waiting for other players" line (every language's
/// `ID_FCORE_*_PC.arc` that has it, once, keeping `.arc.orig`).
fn prepare_lobby_text() {
    let dir = crate::exe_dir().join(r"nativePC\arcLp2Xml");
    let Ok(files) = std::fs::read_dir(&dir) else { return };
    for f in files.flatten().map(|f| f.path()) {
        let name = f.file_name().unwrap_or_default().to_string_lossy().to_string();
        let orig = f.with_extension("arc.orig");
        if !(name.starts_with("ID_FCORE_") && name.ends_with("_PC.arc")) || orig.exists() {
            continue;
        }
        let run = || -> Result<bool, String> {
            let mut entries = lp2_arc::read(&std::fs::read(&f).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            let Some(e) = entries.iter_mut().find(|e| e.type_hash == MES && e.name.eq_ignore_ascii_case(r"message\msgPC\MsgOnline")) else {
                return Ok(false);
            };
            let Some(data) = replace_message(&e.data, WAITING, WAITING_HINT) else { return Ok(false) };
            e.data = data;
            let packed = lp2_arc::write(&entries).map_err(|e| e.to_string())?;
            std::fs::copy(&f, &orig).map_err(|e| e.to_string())?;
            std::fs::write(&f, packed).map_err(|e| e.to_string())?;
            Ok(true)
        };
        match run() {
            Ok(true) => log!("bots: lobby hint added to {name}"),
            Ok(false) => {}
            Err(e) => log!("bots: {name}: {e}"),
        }
    }
}

/// The online maps present (`aArea_ONLINE_NN.arc`), as their numbers.
fn online_maps(dir: &Path) -> Vec<String> {
    let mut maps: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|f| f.file_name().to_str()?.strip_prefix("aArea_ONLINE_")?.strip_suffix(".arc").map(String::from))
        .filter(|m| m.len() == 2 && m.bytes().all(|b| b.is_ascii_digit()))
        .collect();
    maps.sort();
    maps
}

/// Gives every versus map its AI data (once; generating takes up to ~2 s per map).
pub fn prepare() {
    let dir = crate::exe_dir().join(r"nativePC\arcXmlTmpPC");
    for online in online_maps(&dir) {
        match prepare_one(&dir, &online) {
            Ok(true) => log!("bots: aArea_ONLINE_{online} now has AI data"),
            Ok(false) => {}
            Err(e) => log!("bots: aArea_ONLINE_{online}: {e}"),
        }
    }
    prepare_lobby_text();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A script list: header with an empty definition block, root, then `entries`.
    fn xfs(entries: &[(u32, &str)]) -> Vec<u8> {
        let mut body = vec![1, 0, 0, 0]; // mAutoDelete count
        body.push(0);
        body.extend((entries.len() as u32).to_le_bytes());
        for (i, (t, name)) in entries.iter().enumerate() {
            let mut rec = ai_record(*t, name);
            rec[2..4].copy_from_slice(&(i as u16 + 1).to_le_bytes());
            body.extend(rec);
        }
        let mut d = b"XFS\0".to_vec();
        d.extend([0u8; 8]);
        d.extend(0u32.to_le_bytes()); // definitions: none, so the root is at 0x10
        d.extend(1u32.to_le_bytes());
        d.extend((body.len() as u32 + 4).to_le_bytes());
        d.extend(body);
        d
    }

    #[test]
    fn message_replaced_once() {
        let table = [&b"MES\0"[..], &mes_entry("a"), &mes_entry(WAITING), &mes_entry("b")].concat();
        let out = replace_message(&table, WAITING, WAITING_HINT).unwrap();
        assert_eq!(out, [&b"MES\0"[..], &mes_entry("a"), &mes_entry(WAITING_HINT), &mes_entry("b")].concat());
        assert!(replace_message(&[&table[..], &mes_entry(WAITING)].concat(), WAITING, "x").is_none(), "ambiguous");
    }

    #[test]
    fn ai_entries_fill_empty_slots() {
        assert_eq!(ai_record(0, "").len(), EMPTY_LEN, "an empty slot is a record without type and name");
        let online = xfs(&[(2, r"scr\a"), (0, ""), (0, ""), (0, ""), (0, "")]);
        let training = xfs(&[(2, r"scr\b"), (0x1d, r"tool\AI\NaviMesh\T\List_T"), (0x1e, r"tool\AI\NaviMesh\T\Way_T"), (0x1f, r"tool\AI\AreaAI\T")]);
        let out = add_ai_entries(&online, &ai_records(&training).unwrap()).expect("patched");
        let (_, recs) = records(&out).unwrap();
        assert_eq!(recs.iter().map(|r| r.2).collect::<Vec<_>>(), [2, 0x1d, 0x1e, 0x1f, 0]);
        // object indices stay those of the slots
        assert_eq!(recs.iter().map(|r| u16::from_le_bytes([out[r.0 + 2], out[r.0 + 3]])).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
        assert!(add_ai_entries(&xfs(&[(2, "x")]), &ai_records(&training).unwrap()).is_none(), "no free slots");
    }

    /// The real Training lists' AI entries are exactly what `ai_record` builds, and every online
    /// map's script list takes the generated entries. Skipped when the game is not installed.
    #[test]
    fn real_script_lists() {
        let dir = Path::new(r"G:\Lost Planet 2\nativePC\arcXmlTmpPC");
        if !dir.exists() {
            return eprintln!("game not found, skipping");
        }
        for (_, training) in PAIRS {
            let tr = read_arc(&dir.join(format!("aArea_TUTOREAL_{training}.arc"))).unwrap();
            let recs = ai_records(&tr[script_list(&tr, &format!("aArea_TUTOREAL_{training}")).unwrap()].data).unwrap();
            for (r, t) in recs.iter().zip(AI_TYPES) {
                let name = String::from_utf8_lossy(&r[20..r.len() - 33]).into_owned();
                let mut built = ai_record(t, &name);
                built[2..4].copy_from_slice(&r[2..4]);
                assert_eq!(&built, r, "{training}: {name}");
            }
        }
        // End to end on a copy of map 05: the patched archive lists and holds the generated files.
        let tmp = std::env::temp_dir().join(format!("lp2botmaps{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("variable")).unwrap();
        let orig05 = dir.join("aArea_ONLINE_05.arc.orig");
        std::fs::copy(if orig05.exists() { orig05 } else { dir.join("aArea_ONLINE_05.arc") }, tmp.join("aArea_ONLINE_05.arc")).unwrap();
        std::fs::copy(dir.join(r"variable\aArea_ONLINE_05_v.arc"), tmp.join(r"variable\aArea_ONLINE_05_v.arc")).unwrap();
        assert_eq!(prepare_one(&tmp, "05"), Ok(true));
        assert_eq!(prepare_one(&tmp, "05"), Ok(false), "once");
        let es = read_arc(&tmp.join("aArea_ONLINE_05.arc")).unwrap();
        let recs = ai_records(&es[script_list(&es, "aArea_ONLINE_05").unwrap()].data).unwrap();
        assert_eq!(recs.len(), 3);
        for (r, t) in recs.iter().zip(AI_TYPES) {
            let name = String::from_utf8_lossy(&r[20..r.len() - 33]).into_owned();
            let e = es.iter().find(|e| e.name == name).unwrap_or_else(|| panic!("{name} in the archive"));
            match t {
                0x1D => drop(lp2_arc::navi::NavigationList::parse(&e.data).unwrap()),
                0x1E => drop(lp2_arc::navi::WayPoints::parse(&e.data).unwrap()),
                _ => drop(lp2_arc::navi::IntelligenceBox::parse(&e.data).unwrap()),
            }
        }
        std::fs::remove_dir_all(&tmp).unwrap();
        let maps = online_maps(dir);
        assert_eq!(maps.len(), 14);
        for m in maps {
            // The patched archives keep the untouched original beside them.
            let orig = dir.join(format!("aArea_ONLINE_{m}.arc.orig"));
            let es = read_arc(&if orig.exists() { orig } else { dir.join(format!("aArea_ONLINE_{m}.arc")) }).unwrap();
            let list = &es[script_list(&es, &format!("aArea_ONLINE_{m}")).unwrap()].data;
            let recs: Vec<_> = AI_TYPES.iter().map(|&t| ai_record(t, r"tool\AI\NaviMesh\Online_00\List_Online_00")).collect();
            assert!(add_ai_entries(list, &recs).is_some(), "{m}: script list takes the AI entries");
        }
    }
}
