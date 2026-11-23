mod game;
mod tray;
mod wallpaper;
#[cfg(windows)]
mod workshop;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 应用级共享进程状态，避免重复启动
        .manage(game::GameProcess::default())
        // 插件，文件选择器、对话框
    // .plugin(tauri_plugin_dialog::init())  提供系统对话框
    // .plugin(tauri_plugin_opener::init()) 提供打开网页、文件等能力
    // 后面继续接项目原有的 .setup(...) 和 .run(...)
        .plugin(tauri_plugin_dialog::init())
        // 托盘图标和菜单
        .setup(|app| {
            tray::setup(app)?;
            wallpaper::restore_app_icon(app.handle());
            Ok(())
        })
        .on_window_event(tray::on_window_event)
        .invoke_handler(tauri::generate_handler![
            game::game_status,
            game::choose_game_path,
            game::launch_game,
            wallpaper::get_media,
            wallpaper::choose_media,
            wallpaper::reset_media,
            #[cfg(windows)]
            workshop::import_workshop_wallpaper,
            #[cfg(windows)]
            workshop::wallpaper_video_crop
        ])
        .run(tauri::generate_context!())
        .expect("error while running the game launcher");
}
