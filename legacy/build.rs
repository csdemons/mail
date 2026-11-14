// Обработчик команд — из общего списка src-tauri/commands.list (см. build_commands.rs).

#[path = "../src-tauri/build_commands.rs"]
mod build_commands;

fn main() {
    let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let commands = build_commands::read_commands(&dir.join("../src-tauri/commands.list"), &dir.join("../src-tauri/src"));
    build_commands::write_handler(&commands);
    tauri_build::build()
}
