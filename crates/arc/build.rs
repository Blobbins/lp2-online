fn main() {
    let src = ["Recast", "RecastAlloc", "RecastArea", "RecastAssert", "RecastContour", "RecastFilter", "RecastMesh", "RecastMeshDetail", "RecastRasterization", "RecastRegion"];
    cc::Build::new()
        .cpp(true)
        .include("recast/Include")
        .files(src.iter().map(|s| format!("recast/Source/{s}.cpp")))
        .file("recast/wrap/lp2recast.cpp")
        .define("NDEBUG", None)
        .warnings(false)
        .compile("lp2recast");
    println!("cargo:rerun-if-changed=recast");
}
