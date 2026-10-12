//! Live stats (LP2's online rankings: `NetworkRankingLive`, TrueSkill).
//!
//! The game parses XUserReadStats results without checking counts, so every requested view,
//! row and column must exist. Values are placeholders until stats are stored (see the log
//! lines for the view/column ids LP2 uses).

use crate::types::*;
use crate::{log, user, CONFIG, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};

const XUSER_DATA_TYPE_INT64: u8 = 2;
const XUSER_DATA_TYPE_DOUBLE: u8 = 3;
// Standard Live TrueSkill column ids.
const SKILL: u16 = 61; // 62 = games played (0 by default)
const MU: u16 = 63;
const SIGMA: u16 = 64;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Spec {
    pub view_id: u32,
    pub column_count: u32,
    pub columns: [u16; 64],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct View {
    view_id: u32,
    total_rows: u32,
    row_count: u32,
    rows: u32,
}

#[repr(C, align(8))]
#[derive(Clone, Copy, Default)]
struct Row {
    xuid: [u8; 8],
    rank: u32,
    _pad: u32,
    rating: [u8; 8],
    gamertag: [u8; 16],
    column_count: u32,
    columns: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Column {
    id: u16,
    _pad: [u8; 6],
    value: XUserData,
}

const _: () = {
    assert!(size_of::<Spec>() == 136);
    assert!(size_of::<View>() == 16);
    assert!(size_of::<Row>() == 48);
    assert!(size_of::<Column>() == 24);
};

/// A new player's value for a column.
fn placeholder(column: u16) -> XUserData {
    let (kind, value) = match column {
        SKILL => (XUSER_DATA_TYPE_INT64, 1i64.to_le_bytes()),
        MU => (XUSER_DATA_TYPE_DOUBLE, 3.0f64.to_le_bytes()),
        SIGMA => (XUSER_DATA_TYPE_DOUBLE, 1.0f64.to_le_bytes()),
        _ => (XUSER_DATA_TYPE_INT64, 0i64.to_le_bytes()),
    };
    XUserData { kind, _pad: [0; 7], value }
}

fn gamertag(xuid: u64) -> [u8; 16] {
    let name = if xuid == user::xuid() { CONFIG.name.clone() } else { format!("{:08X}", xuid as u32) };
    let mut out = [0u8; 16];
    out[..name.len().min(15)].copy_from_slice(&name.as_bytes()[..name.len().min(15)]);
    out
}

/// XUSER_STATS_READ_RESULTS: one view per spec, one row per xuid, every requested column.
fn results(out: &mut Out, xuids: &[u64], specs: &[Spec]) {
    let header = out.put(CountPtr::default());
    let views = out.array::<View>(specs.len());
    for (v, spec) in specs.iter().enumerate() {
        let columns = &spec.columns[..(spec.column_count as usize).min(64)];
        let rows = out.array::<Row>(xuids.len());
        for (r, &xuid) in xuids.iter().enumerate() {
            let cols = out.array::<Column>(columns.len());
            for (c, &id) in columns.iter().enumerate() {
                unsafe { set(cols, c, Column { id, value: placeholder(id), ..Default::default() }) };
            }
            let row = Row {
                xuid: xuid.to_le_bytes(),
                rank: r as u32 + 1,
                gamertag: gamertag(xuid),
                column_count: columns.len() as u32,
                columns: cols as u32,
                ..Default::default()
            };
            unsafe { set(rows, r, row) };
        }
        let view = View { view_id: spec.view_id, total_rows: xuids.len() as u32, row_count: xuids.len() as u32, rows: rows as u32 };
        unsafe { set(views, v, view) };
    }
    unsafe { set(header, 0, CountPtr { count: specs.len() as u32, ptr: views as u32 }) };
}

pub unsafe fn read(xuids: &[u64], specs: &[Spec], size: *mut u32, buf: *mut u8) -> u32 {
    for s in specs {
        log!("XUserReadStats view {:#x} columns {:?} for {} player(s)", s.view_id, &s.columns[..(s.column_count as usize).min(64)], xuids.len());
    }
    let mut m = Out::measure();
    results(&mut m, xuids, specs);
    if buf.is_null() || *size < m.len as u32 {
        *size = m.len as u32;
        return ERROR_INSUFFICIENT_BUFFER;
    }
    results(&mut Out::new(buf), xuids, specs);
    ERROR_SUCCESS
}

/// XSESSION_VIEW_PROPERTIES as the game passes it to XSessionWriteStats.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ViewProperties {
    pub view_id: u32,
    pub property_count: u32,
    pub properties: *const XUserProperty,
}

/// shortcut: stats writes are only logged; store them (locally or on the master) to make
/// rankings and stat-based progression real.
pub unsafe fn log_write(xuid: u64, views: &[ViewProperties]) {
    for v in views {
        let props = if v.properties.is_null() { &[][..] } else { std::slice::from_raw_parts(v.properties, v.property_count as usize) };
        let shown: Vec<String> = props.iter().map(|p| format!("{:#x}:{}={:02x?}", p.id, p.data.kind, p.data.value)).collect();
        log!("XSessionWriteStats {xuid:016x} view {:#x}: {}", v.view_id, shown.join(" "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_view_row_and_column_exists() {
        let mut spec = Spec { view_id: 7, column_count: 2, columns: [0; 64] };
        spec.columns[0] = MU;
        spec.columns[1] = 1;
        let mut size = 0;
        assert_eq!(unsafe { read(&[1, 2], &[spec], &mut size, std::ptr::null_mut()) }, ERROR_INSUFFICIENT_BUFFER);
        let mut buf = vec![0u64; size as usize / 8 + 1];
        assert_eq!(unsafe { read(&[1, 2], &[spec], &mut size, buf.as_mut_ptr() as _) }, ERROR_SUCCESS);
        unsafe {
            let h = *(buf.as_ptr() as *const CountPtr);
            let v = *(h.ptr as *const View);
            assert_eq!((h.count, v.view_id, v.row_count), (1, 7, 2));
            let row = *((v.rows as *const Row).add(1));
            assert_eq!((u64::from_le_bytes(row.xuid), row.rank, row.column_count), (2, 2, 2));
            let col = *(row.columns as *const Column);
            assert_eq!((col.id, col.value.kind, f64::from_le_bytes(col.value.value)), (MU, XUSER_DATA_TYPE_DOUBLE, 3.0));
        }
    }
}
