// Nouve runs without a console window: Nouve is the whole UI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    nouve_lib::run()
}
