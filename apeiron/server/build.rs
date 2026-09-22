fn main() {
    let dist = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../web/dist");
    if dist.is_dir() {
        println!("cargo:rustc-cfg=embed_frontend");
    } else {
        println!("cargo:warning=Apeiron web dist not found at {dist:?}; API-only build");
    }
}
