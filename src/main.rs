#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// 桌面端二进制入口：真正的应用逻辑在 src/lib.rs 的 run() 中（移动端共用同一份代码，
// 通过 #[cfg_attr(mobile, tauri::mobile_entry_point)] 导出移动端入口）。
fn main() {
    web2app_lib::run()
}
