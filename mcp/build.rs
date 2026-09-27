//! Stamp the build time into the binary so a running server can tell a client
//! it is out of date. Two agents lost most of a session to a stale server whose
//! compiled-in docs described a syntax its compiled-in parser did not accept.
//!
//! Also compile the example songs in: a server installed with `cargo install`
//! runs far from the repo, and its examples are half of what it teaches.

fn main() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=SYNTH_BUILD_EPOCH={}", now);
    println!("cargo:rustc-env=SYNTH_BUILD_TIME={}", now);
    // Rebuild the stamp whenever anything it describes changes.
    println!("cargo:rerun-if-changed=../core/src");
    println!("cargo:rerun-if-changed=../mcp/src");
    println!("cargo:rerun-if-changed=../docs");

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<_> = std::fs::read_dir(&dir).expect("examples/")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("synth"))
        .collect();
    names.sort();
    let mut out = String::from("&[\n");
    for path in names {
        let path = path.canonicalize().expect("example path");
        let stem = path.file_stem().and_then(|s| s.to_str()).expect("example name");
        out += &format!("    ({:?}, include_str!({:?})),\n", stem, path.display().to_string());
    }
    out += "]\n";
    let dest = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("examples.rs");
    std::fs::write(dest, out).expect("write examples.rs");
}
