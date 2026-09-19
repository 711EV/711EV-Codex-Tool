fn main() {
    println!("cargo:rerun-if-changed=resources/mcp");
    tauri_build::build()
}
