// A release build on Windows must not open a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ampiera_device_simulator_lib::run();
}
