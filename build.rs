//! Build script: keeps Cargo reliably re-running when the script itself changes.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
}
