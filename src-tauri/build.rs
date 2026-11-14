// Команды приложения объявлены явно: каждая получает разрешение `allow-<команда>`,
// и окно может вызывать только то, что перечислено в его capabilities
// (главное окно — всё, окно письма — только нужное письму, окна редактора — ничего).
// Список команд — один: commands.list (см. build_commands.rs).

#[path = "build_commands.rs"]
mod build_commands;

fn main() {
    let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let commands = build_commands::read_commands(&dir.join("commands.list"), &dir.join("src"));
    build_commands::write_handler(&commands);
    build_commands::write_capabilities(&commands, &dir.join("capabilities"));
    // tauri-build ждёт &'static [&str]; build-скрипт живёт недолго — утечка не страшна.
    let names: &'static [&'static str] = Box::leak(commands.iter().map(|c| &*Box::leak(c.name.clone().into_boxed_str())).collect::<Vec<&'static str>>().into_boxed_slice());
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(names)))
        .expect("failed to run tauri-build");
}
