//! cargo run -p lp2-arc --release --example check_all -- "<game>/nativePC"
//! Reads every .arc under the folder and checks read(write(read(x))) == read(x).
fn main() {
    let root = std::env::args().nth(1).expect("path to nativePC");
    let (mut ok, mut bad) = (0, 0);
    let mut stack = vec![std::path::PathBuf::from(root)];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("arc")) {
                let r = lp2_arc::read(&std::fs::read(&p).unwrap())
                    .and_then(|a| lp2_arc::read(&lp2_arc::write(&a)?).map(|b| a == b));
                match r {
                    Ok(true) => ok += 1,
                    other => {
                        bad += 1;
                        println!("{}: {:?}", p.display(), other.map(|_| "roundtrip differs"));
                    }
                }
            }
        }
    }
    println!("{ok} ok, {bad} failed");
}
