//! lp2-navi obj <stage.arc> <out.obj>   navigation mesh + way points of a stage as Wavefront OBJ
//! lp2-navi info <stage.arc>            counts of its navigation, way point and AI box data

use lp2_arc::navi::{self, IntelligenceBox, NavigationList, WayPoints};
use std::process::ExitCode;

fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let (cmd, arc) = match args {
        [c, a, ..] => (c.as_str(), a),
        _ => return Err("usage: lp2-navi obj <stage.arc> <out.obj> | info <stage.arc>".into()),
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
        _ => return Err("usage: lp2-navi obj <stage.arc> <out.obj> | info <stage.arc>".into()),
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
