use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager, WebviewWindow, image::Image};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

const WALLPAPER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "mp4", "webm"];
const ICON_EXTENSIONS: &[&str] = &["png", "ico"];
const MUSIC_EXTENSIONS: &[&str] = &["mp3", "wav", "flac", "m4a"];
static MEDIA_WRITE: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaSlot {
    Wallpaper,
    Icon,
    Music,
}

impl MediaSlot {
    fn directory(self, app: &AppHandle) -> Result<PathBuf, String> {
        let name = match self {
            Self::Wallpaper => "wallpaper",
            Self::Icon => "icon",
            Self::Music => "music",
        };
        app.path()
            .app_local_data_dir()
            .map(|dir| dir.join("custom-media").join(name))
            .map_err(format_error)
    }

    fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Wallpaper => WALLPAPER_EXTENSIONS,
            Self::Icon => ICON_EXTENSIONS,
            Self::Music => MUSIC_EXTENSIONS,
        }
    }
}

#[derive(Serialize)]
pub struct Media {
    path: PathBuf,
    kind: &'static str,
}

fn media_kind(slot: MediaSlot, extension: &str) -> &'static str {
    if matches!(slot, MediaSlot::Music) {
        return "audio";
    }
    match extension {
        "mp4" | "webm" => "video",
        _ => "image",
    }
}

fn current(directory: &Path, slot: MediaSlot) -> io::Result<Option<Media>> {
    let filename = match fs::read_to_string(directory.join("selected")) {
        Ok(filename) => filename,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some((stem, extension)) = filename.trim().split_once('.') else {
        return Ok(None);
    };
    if stem.len() != 32
        || !stem.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !slot.extensions().contains(&extension)
    {
        return Ok(None);
    }
    let path = directory.join(filename.trim());
    Ok(path.is_file().then(|| Media {
        path,
        kind: media_kind(slot, extension),
    }))
}

/// 完整复制并检查新文件后才替换选择；原始文件和旧选择在失败时保留。
fn import(directory: &Path, source: &Path, slot: MediaSlot) -> Result<Media, String> {
    let mut input = BufReader::new(File::open(source).map_err(format_error)?);
    if !input.get_ref().metadata().map_err(format_error)?.is_file() {
        return Err("请选择本地媒体文件。".into());
    }
    let extension = infer::get(input.fill_buf().map_err(format_error)?)
        .map(|media| media.extension())
        .filter(|extension| slot.extensions().contains(extension))
        .ok_or_else(|| match slot {
            MediaSlot::Wallpaper => "请选择 JPG、PNG、WebP、GIF、MP4 或 WebM 文件。".to_string(),
            MediaSlot::Icon => "请选择 PNG 或 ICO 文件。".to_string(),
            MediaSlot::Music => "请选择 MP3、WAV、FLAC 或 M4A 音乐文件。".to_string(),
        })?;
    let _guard = MEDIA_WRITE.lock().map_err(format_error)?;
    fs::create_dir_all(directory).map_err(format_error)?;
    let previous = current(directory, slot).map_err(format_error)?;
    let token = Uuid::new_v4().simple().to_string();
    let filename = format!("{token}.{extension}");
    let path = directory.join(&filename);
    let pending = directory.join(format!("{token}.pending"));
    let temporary_selection = directory.join(format!("{token}.selection"));
    let result = (|| -> Result<(), String> {
        let mut output = File::create(&pending).map_err(format_error)?;
        io::copy(&mut input, &mut output).map_err(format_error)?;
        output.sync_all().map_err(format_error)?;
        drop(output);
        if matches!(slot, MediaSlot::Icon) {
            Image::from_path(&pending).map_err(|error| format!("无法读取图标：{error}"))?;
        }
        fs::rename(&pending, &path).map_err(format_error)?;
        fs::write(&temporary_selection, &filename).map_err(format_error)?;
        fs::rename(&temporary_selection, directory.join("selected")).map_err(format_error)
    })();
    if let Err(error) = result {
        for staged in [&pending, &path, &temporary_selection] {
            let _ = fs::remove_file(staged);
        }
        return Err(error);
    }
    if let Some(previous) = previous {
        let _ = fs::remove_file(previous.path);
    }
    Ok(Media {
        path,
        kind: media_kind(slot, extension),
    })
}

#[cfg(windows)]
pub(crate) fn import_wallpaper(app: &AppHandle, source: &Path) -> Result<Media, String> {
    import(&MediaSlot::Wallpaper.directory(app)?, source, MediaSlot::Wallpaper)
}

#[cfg(windows)]
pub(crate) fn selected_wallpaper_video(app: &AppHandle) -> Option<PathBuf> {
    let media = get_media(app.clone(), MediaSlot::Wallpaper).ok()??;
    (media.kind == "video").then_some(media.path)
}

fn reset(directory: &Path, slot: MediaSlot) -> Result<(), String> {
    let _guard = MEDIA_WRITE.lock().map_err(format_error)?;
    let previous = current(directory, slot).map_err(format_error)?;
    match fs::remove_file(directory.join("selected")) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format_error(error)),
    }
    if let Some(previous) = previous {
        let _ = fs::remove_file(previous.path);
    }
    Ok(())
}

fn format_error(error: impl std::fmt::Display) -> String {
    format!("无法访问本地媒体文件：{error}")
}

fn apply_app_icon(app: &AppHandle, path: Option<&Path>) -> Result<(), String> {
    let image = match path {
        Some(path) => Image::from_path(path).map_err(format_error)?,
        None => app
            .default_window_icon()
            .cloned()
            .ok_or("找不到默认程序图标。")?,
    };
    if let Some(window) = app.get_webview_window("main") {
        window.set_icon(image.clone()).map_err(format_error)?;
    }
    if let Some(tray) = app.tray_by_id("launcher") {
        tray.set_icon(Some(image)).map_err(format_error)?;
    }
    Ok(())
}

pub fn restore_app_icon(app: &AppHandle) {
    if let Err(error) = get_media(app.clone(), MediaSlot::Icon)
        .and_then(|media| media.map_or(Ok(()), |icon| apply_app_icon(app, Some(&icon.path))))
    {
        eprintln!("{error}");
    }
}

#[tauri::command]
pub fn get_media(app: AppHandle, slot: MediaSlot) -> Result<Option<Media>, String> {
    current(&slot.directory(&app)?, slot).map_err(format_error)
}

/// 从原生选择器导入文件，返回可由资源协议读取的应用数据目录副本。
#[tauri::command]
pub async fn choose_media(
    app: AppHandle,
    window: WebviewWindow,
    slot: MediaSlot,
) -> Result<Option<Media>, String> {
    let directory = slot.directory(&app)?;
    let title = match slot {
        MediaSlot::Wallpaper => "选择原神背景",
        MediaSlot::Icon => "选择软件图标",
        MediaSlot::Music => "选择背景音乐",
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        let selected = window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title(title)
            .add_filter(title, slot.extensions())
            .blocking_pick_file();
        let Some(selected) = selected else {
            return Ok(None);
        };
        let source = selected.into_path().map_err(format_error)?;
        import(&directory, &source, slot).map(Some)
    })
    .await
    .map_err(format_error)??;
    if matches!(slot, MediaSlot::Icon)
        && let Some(icon) = &result
    {
        apply_app_icon(&app, Some(&icon.path))?;
    }
    Ok(result)
}

#[tauri::command]
pub fn reset_media(app: AppHandle, slot: MediaSlot) -> Result<(), String> {
    reset(&slot.directory(&app)?, slot)?;
    if matches!(slot, MediaSlot::Icon) {
        apply_app_icon(&app, None)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_and_reset_preserve_the_source() {
        let workspace = std::env::temp_dir().join(format!("launcher-wallpaper-{}", Uuid::new_v4()));
        fs::create_dir_all(&workspace).unwrap();
        let directory = workspace.join("game");
        let image = workspace.join("fixture.png");
        let invalid = workspace.join("invalid.png");
        fs::write(&image, b"\x89PNG\r\n\x1a\n").unwrap();
        fs::write(&invalid, b"not an image").unwrap();

        assert!(current(&directory, MediaSlot::Wallpaper).unwrap().is_none());
        let first = import(&directory, &image, MediaSlot::Wallpaper).unwrap();
        assert_eq!(first.kind, "image");
        assert!(import(&directory, &invalid, MediaSlot::Wallpaper).is_err());
        assert_eq!(
            current(&directory, MediaSlot::Wallpaper)
                .unwrap()
                .unwrap()
                .path,
            first.path
        );

        let second = import(&directory, &image, MediaSlot::Wallpaper).unwrap();
        assert_ne!(second.path, first.path);
        assert!(!first.path.exists());
        assert_eq!(
            current(&directory, MediaSlot::Wallpaper)
                .unwrap()
                .unwrap()
                .path,
            second.path
        );
        reset(&directory, MediaSlot::Wallpaper).unwrap();
        assert!(current(&directory, MediaSlot::Wallpaper).unwrap().is_none());
        assert!(!second.path.exists());
        assert!(image.is_file());
        fs::remove_file(image).unwrap();
        fs::remove_file(invalid).unwrap();
        fs::remove_dir(directory).unwrap();
        fs::remove_dir(workspace).unwrap();
    }
}
