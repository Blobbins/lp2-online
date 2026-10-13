//! lp2-navi obj <stage.arc> <out.obj>   navigation mesh + way points of a stage as Wavefront OBJ
//! lp2-navi info <stage.arc>            counts of its navigation, way point and AI box data
//! lp2-navi gen <stage_v.arc> <out dir> generates List_/Way_/AreaAI from the stage's walking collision,
//!                                      named as `lp2-arc extract` names files, plus navmesh.obj

use lp2_arc::navgen;
use lp2_arc::navi::{self, IntelligenceBox, NavigationList, WayPoints};
use std::process::ExitCode;

const USAGE: &str = "usage: lp2-navi obj <stage.arc> <out.obj> | info <stage.arc> | gen <stage_v.arc> <out dir>";

fn generate(v_arc: &str, out: &str) -> Result<(), Box<dyn std::error::Error>> {
    // aArea_ONLINE_05_v.arc -> stage aArea_ONLINE_05, folder Online_05
    let file = std::path::Path::new(v_arc).file_name().unwrap_or_default().to_string_lossy().into_owned();
    let stage = file.strip_suffix("_v.arc").ok_or("expected a <stage>_v.arc file")?;
    let dir = format!("Online_{}", stage.rsplit('_').next().unwrap_or(stage));
    let entries = lp2_arc::read(&std::fs::read(v_arc)?)?;
    let c = lp2_arc::sbc::parse(&navgen::walking_collision(&entries).ok_or(r"no scr\..\atari collision")?.data)?;
    let t = std::time::Instant::now();
    let (list, way, ibl) = navgen::generate(&c, &dir, &navgen::PARAMS)?;
    let m = &list.entries[0].mesh;
    println!("{stage}: {} triangles, {} way points in {} ms", m.polygons.len(), way.nodes.len(), t.elapsed().as_millis());
    let out = std::path::Path::new(out);
    for e in navgen::entries(stage, &dir, &list, &way, &ibl)? {
        let p = out.join(format!("{}.{:08x}.{:x}", e.name.replace('\\', "/"), e.type_hash, e.flags >> 29));
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(p, e.data)?;
    }
    std::fs::write(out.join("navmesh.obj"), navi::to_obj(&list, Some(&way)))?;
    Ok(())
}

fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let (cmd, arc) = match args {
        [c, v_arc, out] if c == "gen" => return generate(v_arc, out),
        [c, a, ..] => (c.as_str(), a),
        _ => return Err(USAGE.into()),
    };
    let entries = lp2_arc::read(&std::fs::read(arc)?)?;
    let find = |t: u32| entries.iter().find(|e| e.type_hash == t).map(|e| &e.data[..]);
    let list = NavigationList::parse(find(navi::LIST_TYPE).ok_or("no navigation mesh list in this archive")?)?;
    let way = find(navi::WAY_TYPE).map(WayPoints::parse).transpose()?;
    match (cmd, args.get(2)) {
        ("obj", Some(out)) => std::fs::write(out, navi::to_obj(&list, way.as_ref()))?,
        ("info", None) => {
            for e in &list.entries {
                let m = &e.mesh;
                let links: usize = m.polygons.iter().map(|p| p.links.len()).sum();
                println!("{}: {} vertices, {} polygons, {links} links, {} areas", e.path, m.vertices.len(), m.polygons.len(), m.areas.len());
            }
            if let Some(w) = &way {
                println!("way points: {}", w.nodes.len());
            }
            if let Some(b) = find(navi::IBL_TYPE).map(IntelligenceBox::parse).transpose()? {
                println!("AI boxes: {}", b.boxes.len());
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lp2-navi: {e}");
            ExitCode::FAILURE
        }
    }
}
