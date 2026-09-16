//! Stamp the build time into the binary so a running server can tell a client
//! it is out of date. Two agents lost most of a session to a stale server whose
//! compiled-in docs described a syntax its compiled-in parser did not accept.

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
}
