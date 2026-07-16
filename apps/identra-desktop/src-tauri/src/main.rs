// Prevent a console window on Windows release builds; no-op elsewhere.
#![cfg_attr(
    not(debug_assertions),
    cfg_attr(windows, windows_subsystem = "windows")
)]

fn main() {
    identra_desktop::run();
}
