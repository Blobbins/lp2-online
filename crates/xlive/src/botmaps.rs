//! Bots need a map's AI navigation data, which only the Training ("AI Enemy Battle") versions of
//! the versus maps ship. Training stage aArea_TUTOREAL_NN is an online map plus three script-list
//! entries (Tool\ScrList, an XFS file): mType 0x1d `tool\AI\NaviMesh\..\List_..` (navigation mesh
//! list), 0x1e `..\Way_..` (way points), 0x1f `tool\AI\AreaAI\aArea_TUTOREAL_NN` (area AI).
//!
//! `prepare` copies those resources into the online stage archive and puts the three entries into
//! empty slots of the online script list. The original archive is kept as `<name>.arc.orig`; an
//! archive that already has one is left alone.

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

/// The online script list with the training list's three AI entries in its first empty slots.
pub fn add_ai_entries(online: &[u8], training: &[u8]) -> Option<Vec<u8>> {
    let (root, ons) = records(online)?;
    let (_, trs) = records(training)?;
    let ai: Vec<&[u8]> = trs.iter().filter(|r| AI_TYPES.contains(&r.2)).map(|&(o, n, _)| &training[o..o + n]).collect();
    let empty: Vec<_> = ons.iter().filter(|r| r.1 == EMPTY_LEN && r.2 == 0).take(3).collect();
    if ai.len() != 3 || empty.len() != 3 {
        return None;
    }
    let mut out = online.to_vec();
    // Back to front so earlier offsets stay valid; each entry keeps the slot's object index.
    for (&&(o, n, _), rec) in empty.iter().zip(&ai).rev() {
        let mut rec = rec.to_vec();
        rec[2..4].copy_from_slice(&online[o + 2..o + 4]);
        out.splice(o..o + n, rec);
    }
    let size = (out.len() - root - 4) as u32;
    out[root + 4..root + 8].copy_from_slice(&size.to_le_bytes());
    records(&out).map(|_| out)
}

fn prepare_one(dir: &Path, online: &str, training: &str) -> Result<bool, String> {
    let arc = dir.join(format!("aArea_ONLINE_{online}.arc"));
    let orig = dir.join(format!("aArea_ONLINE_{online}.arc.orig"));
    if orig.exists() {
        return Ok(false);
    }
    let read = |p: &Path| std::fs::read(p).map_err(|e| e.to_string()).and_then(|b| lp2_arc::read(&b).map_err(|e| e.to_string()));
    let mut entries = read(&arc)?;
    let tr = read(&dir.join(format!("aArea_TUTOREAL_{training}.arc")))?;
    let list = |es: &[lp2_arc::Entry], name: &str| es.iter().position(|e| e.type_hash == SCRLIST && e.name.eq_ignore_ascii_case(name));
    let on_i = list(&entries, &format!(r"Tool\ScrList\aArea_ONLINE_{online}")).ok_or("no online script list")?;
    let tr_i = list(&tr, &format!(r"Tool\ScrList\aArea_TUTOREAL_{training}")).ok_or("no training script list")?;
    entries[on_i].data = add_ai_entries(&entries[on_i].data, &tr[tr_i].data).ok_or("script list layout not as expected")?;
    let ai = |e: &&lp2_arc::Entry| {
        let n = e.name.to_ascii_lowercase();
        n.starts_with(r"tool\ai\areaai\") || n.starts_with(r"tool\ai\navimesh\")
    };
    entries.extend(tr.iter().filter(ai).cloned());
    let packed = lp2_arc::write(&entries).map_err(|e| e.to_string())?;
    std::fs::copy(&arc, &orig).map_err(|e| e.to_string())?;
    std::fs::write(&arc, packed).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Gives the four versus maps that have Training versions their AI data (once).
pub fn prepare() {
    let dir = crate::exe_dir().join(r"nativePC\arcXmlTmpPC");
    for (online, training) in PAIRS {
        match prepare_one(&dir, online, training) {
            Ok(true) => log!("bots: aArea_ONLINE_{online} now has the AI data of aArea_TUTOREAL_{training}"),
            Ok(false) => {}
            Err(e) => log!("bots: aArea_ONLINE_{online}: {e}"),
        }
    }
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
            let mut fields = vec![];
            fields.extend(1u32.to_le_bytes());
            fields.extend(t.to_le_bytes());
            fields.extend(1u32.to_le_bytes());
            fields.extend(name.as_bytes());
            fields.push(0);
            for _ in 0..4 {
                fields.extend(1u32.to_le_bytes());
                fields.extend(0u32.to_le_bytes());
            }
            body.extend(3u16.to_le_bytes());
            body.extend((i as u16 + 1).to_le_bytes());
            body.extend((fields.len() as u32 + 4).to_le_bytes());
            body.extend(fields);
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
    fn ai_entries_fill_empty_slots() {
        let online = xfs(&[(2, r"scr\a"), (0, ""), (0, ""), (0, ""), (0, "")]);
        let training = xfs(&[(2, r"scr\b"), (0x1d, r"tool\AI\NaviMesh\T\List_T"), (0x1e, r"tool\AI\NaviMesh\T\Way_T"), (0x1f, r"tool\AI\AreaAI\T")]);
        let out = add_ai_entries(&online, &training).expect("patched");
        let (_, recs) = records(&out).unwrap();
        assert_eq!(recs.iter().map(|r| r.2).collect::<Vec<_>>(), [2, 0x1d, 0x1e, 0x1f, 0]);
        // object indices stay those of the slots
        assert_eq!(recs.iter().map(|r| u16::from_le_bytes([out[r.0 + 2], out[r.0 + 3]])).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
        assert!(add_ai_entries(&xfs(&[(2, "x")]), &training).is_none(), "no free slots");
    }
}
