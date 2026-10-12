fn main() {
    // The game imports xlive.dll by ordinal, so the export table must come from this .def.
    // Only the 32-bit build has the exports; other targets compile to an empty crate.
    let def = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("xlive.def");
    println!("cargo:rerun-if-changed={}", def.display());
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86") {
        println!("cargo:rustc-cdylib-link-arg=/DEF:{}", def.display());
    }
}
