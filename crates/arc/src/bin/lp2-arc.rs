//! lp2-arc list    <file.arc>
//! lp2-arc extract <file.arc> <out dir>     files become `<name>.<type hash>`
//! lp2-arc pack    <dir> <file.arc>         reverse of extract
//! lp2-arc grep    <dir> <text>             which archive files contain `text`

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() { files_under(&p, out)? } else { out.push(p) }
    }
    Ok(())
}

fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    match args {
        [cmd, arc] if cmd == "list" => {
            for e in lp2_arc::read(&std::fs::read(arc)?)? {
                println!("{:08x} {:>9} {}", e.type_hash, e.data.len(), e.name);
            }
        }
        [cmd, arc, out] if cmd == "extract" => {
            for e in lp2_arc::read(&std::fs::read(arc)?)? {
                let rel = format!("{}.{:08x}.{:x}", e.name.replace('\\', "/"), e.type_hash, e.flags >> 29);
                // Archive names come from a file: refuse anything that would escape `out`.
                if rel.split('/').any(|c| c == ".." || c.contains(':')) || rel.starts_with('/') {
                    return Err(format!("unsafe entry name {:?}", e.name).into());
                }
                let p = Path::new(out).join(rel);
                std::fs::create_dir_all(p.parent().unwrap())?;
                std::fs::write(p, e.data)?;
            }
        }
        [cmd, dir, arc] if cmd == "pack" => {
            let mut files = vec![];
            files_under(Path::new(dir), &mut files)?;
            files.sort();
            let mut entries = vec![];
            for f in files {
                let rel = f.strip_prefix(dir)?.to_string_lossy().replace('/', "\\");
                let mut parts = rel.rsplitn(3, '.');
                let (Some(flags), Some(hash), Some(name)) = (parts.next(), parts.next(), parts.next()) else {
                    return Err(format!("{rel}: expected <name>.<type hash>.<flags>").into());
                };
                entries.push(lp2_arc::Entry {
                    name: name.into(),
                    type_hash: u32::from_str_radix(hash, 16)?,
                    flags: u32::from_str_radix(flags, 16)? << 29,
                    data: std::fs::read(&f)?,
                });
            }
            std::fs::write(arc, lp2_arc::write(&entries)?)?;
        }
        [cmd, dir, text] if cmd == "grep" => {
            let mut files = vec![];
            files_under(Path::new(dir), &mut files)?;
            for f in files.iter().filter(|f| f.extension().is_some_and(|x| x.eq_ignore_ascii_case("arc"))) {
                let Ok(entries) = std::fs::read(f).map_err(|e| e.to_string()).and_then(|b| lp2_arc::read(&b).map_err(|e| e.to_string())) else {
                    eprintln!("skipping {}", f.display());
                    continue;
                };
                for e in entries.iter().filter(|e| e.data.windows(text.len()).any(|w| w == text.as_bytes())) {
                    println!("{} :: {} ({:08x})", f.display(), e.name, e.type_hash);
                }
            }
        }
        _ => return Err("usage: lp2-arc list <arc> | extract <arc> <dir> | pack <dir> <arc> | grep <dir> <text>".into()),
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lp2-arc: {e}");
            ExitCode::FAILURE
        }
    }
}
