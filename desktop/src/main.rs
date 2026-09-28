#![windows_subsystem = "windows"]

mod autostart;
mod config;
mod dialog;
mod menu;
mod paint;
mod send;
mod theme;
mod tray;
mod ui;
mod win;

fn main() {
    // A second copy (say, started by hand after sign-in started one) leaves the first alone.
    if win::first_instance() {
        ui::run();
    }
}
