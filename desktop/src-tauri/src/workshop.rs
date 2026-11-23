use crate::wallpaper::{Media, import_wallpaper, selected_wallpaper_video};
use serde::{Deserialize, Serialize};
use std::{
    env, fs, io,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, Url};
use uuid::Uuid;
use windows_sys::Win32::{
    System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW},
    UI::WindowsAndMessaging::FindWindowW,
};

const WORKSHOP_APP: &str = "431960";
const RECORD_START_SECONDS: &str = "6";
const RECORD_SECONDS: &str = "20";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
static IMPORT: Mutex<()> = Mutex::new(());

#[derive(PartialEq, Serialize)]
pub struct VideoCrop {
    width: u32,
    height: u32,
    x: u32,
    y: u32,
}

fn stable_video_crop(log: &str) -> Option<VideoCrop> {
    let samples: Vec<_> = log
        .lines()
        .filter_map(|line| {
            let (_, value) = line.rsplit_once(" crop=")?;
            let values: Vec<u32> = value
                .trim()
                .split(':')
                .map(str::parse)
                .collect::<Result<_, _>>()
                .ok()?;
            let [width, height, x, y] = values.as_slice() else {
                return None;
            };
            Some(VideoCrop {
                width: *width,
                height: *height,
                x: *x,
                y: *y,
            })
        })
        .collect();

    // Only crop borders consistently present in a multi-frame sample.
    if samples.len() < 4 {
        return None;
    }
    let total = samples.len();
    samples
        .iter()
        .position(|crop| {
            crop.width > 0
                && crop.height > 0
                && (crop.x > 0 || crop.y > 0)
                && samples.iter().filter(|other| *other == crop).count() * 5 >= total * 4
        })
        .and_then(|index| samples.into_iter().nth(index))
}

#[tauri::command]
pub async fn wallpaper_video_crop(app: AppHandle) -> Option<VideoCrop> {
    tauri::async_runtime::spawn_blocking(move || {
        let source = selected_wallpaper_video(&app)?;
        let output = quiet(&ffmpeg(&app).ok()?)
            .args([
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "info",
                "-t",
                "8",
                "-i",
            ])
            .arg(source)
            .args([
                "-an",
                "-sn",
                "-dn",
                "-vf",
                "fps=2,cropdetect=limit=18:round=2:reset=1",
                "-f",
                "null",
                "-",
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        stable_video_crop(&String::from_utf8_lossy(&output.stderr))
    })
    .await
    .ok()
    .flatten()
}

#[derive(Deserialize)]
struct WorkshopProject {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    file: String,
}

fn workshop_id(input: &str) -> Result<u64, String> {
    let input = input.trim();
    let candidate = if let Some(id) = input.strip_prefix("steam://url/CommunityFilePage/") {
        id.to_owned()
    } else if let Ok(url) = Url::parse(input) {
        if url.scheme() != "https"
            || url.host_str() != Some("steamcommunity.com")
            || !matches!(
                url.path().trim_end_matches('/'),
                "/sharedfiles/filedetails" | "/workshop/filedetails"
            )
        {
            return Err("请输入 Steam 创意工坊链接或作品 ID。".into());
        }
        url.query_pairs()
            .find(|(key, _)| key == "id")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default()
    } else {
        input.to_owned()
    };
    candidate
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0 && candidate.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| "创意工坊作品 ID 无效。".into())
}

fn wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain([0])
        .collect()
}

fn steam_registry_path() -> Option<PathBuf> {
    let key = wide(r"Software\Valve\Steam");
    let name = wide("SteamPath");
    let mut buffer = [0_u16; 4096];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let end = buffer.iter().position(|unit| *unit == 0)?;
    Some(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..end])))
}

fn steam_libraries() -> Result<Vec<PathBuf>, String> {
    let root = steam_registry_path()
        .or_else(|| env::var_os("ProgramFiles(x86)").map(|base| PathBuf::from(base).join("Steam")))
        .filter(|root| root.join("steamapps").is_dir())
        .ok_or("找不到 Steam 安装目录。")?;
    let mut libraries = vec![root.clone()];
    match fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
        Ok(config) => {
            for line in config.lines() {
                let mut quoted = line.split('"');
                if quoted.nth(1) == Some("path")
                    && let Some(path) = quoted.nth(1)
                {
                    let path = PathBuf::from(path.replace(r"\\", r"\"));
                    if path.is_absolute() && !libraries.contains(&path) {
                        libraries.push(path);
                    }
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("无法读取 Steam 库配置：{error}")),
    }
    Ok(libraries)
}

fn safe_file(folder: &Path, relative: &str) -> Result<PathBuf, String> {
    let file = folder.join(relative);
    let resolved = file.canonicalize().map_err(io_error)?;
    if !resolved.starts_with(folder.canonicalize().map_err(io_error)?) || !resolved.is_file() {
        return Err("壁纸项目引用了无效文件。".into());
    }
    Ok(file)
}

fn io_error(error: io::Error) -> String {
    format!("无法读取或处理壁纸：{error}")
}

fn quiet(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null());
    command
}

fn ffmpeg(app: &AppHandle) -> Result<PathBuf, String> {
    let local = app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("workshop-tools/ffmpeg/ffmpeg.exe");
    if local.is_file() {
        return Ok(local);
    }
    env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("ffmpeg.exe"))
        .find(|file| file.is_file())
        .ok_or_else(|| {
            format!(
                "场景壁纸需要带 gfxcapture 的 FFmpeg。请将 FFmpeg 解压到 {}，或加入 PATH。",
                local.parent().unwrap().display()
            )
        })
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

struct SceneCapture {
    engine: PathBuf,
    window: String,
    output: PathBuf,
}

impl Drop for SceneCapture {
    fn drop(&mut self) {
        if let Ok(mut process) = quiet(&self.engine)
            .args(["-control", "closeWallpaper", "-location", &self.window])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            let _ = wait_for(&mut process, Duration::from_secs(2));
        }
        let _ = fs::remove_file(&self.output);
        let _ = fs::remove_file(self.output.with_extension("progress"));
    }
}

fn wait_for(child: &mut Child, duration: Duration) -> io::Result<std::process::ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if start.elapsed() < duration => thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                stop(child);
                return Err(io::Error::new(io::ErrorKind::TimedOut, "操作超时"));
            }
            Err(error) => {
                stop(child);
                return Err(error);
            }
        }
    }
}

fn preview_handle(window: &str) -> Result<usize, String> {
    let title = wide(window);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(12) {
        let handle = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
        if !handle.is_null() {
            return Ok(handle as usize);
        }
        thread::sleep(Duration::from_millis(150));
    }
    Err("场景播放窗口未打开。请先运行 Wallpaper Engine，确认该壁纸可以独立播放，再重试。".into())
}

fn capture_scene(app: &AppHandle, libraries: &[PathBuf], project: &Path) -> Result<Media, String> {
    let ffmpeg = ffmpeg(app)?;
    let filters = quiet(&ffmpeg)
        .args(["-hide_banner", "-filters"])
        .output()
        .map_err(io_error)?;
    if !filters.status.success()
        || !String::from_utf8_lossy(&filters.stdout)
            .split_whitespace()
            .any(|word| word == "gfxcapture")
    {
        return Err(
            "当前 FFmpeg 不支持窗口录制，请安装带 gfxcapture 的 Windows 版 FFmpeg。".into(),
        );
    }
    let engine = libraries
        .iter()
        .map(|library| library.join("steamapps/common/wallpaper_engine/wallpaper64.exe"))
        .find(|file| file.is_file())
        .ok_or("未找到 Wallpaper Engine。")?;
    let token = Uuid::new_v4().simple().to_string();
    let capture = SceneCapture {
        engine,
        window: format!("LauncherWallpaper{token}"),
        output: env::temp_dir().join(format!("launcher-wallpaper-{token}.mp4")),
    };
    let mut preview = quiet(&capture.engine)
        .args(["-control", "openWallpaper", "-file"])
        .arg(project)
        .args([
            "-playInWindow",
            &capture.window,
            "-width",
            "1920",
            "-height",
            "1080",
            "-borderless",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(io_error)?;
    if !wait_for(&mut preview, Duration::from_secs(8))
        .map_err(|error| format!("请确认 Wallpaper Engine 正在运行：{error}"))?
        .success()
    {
        return Err("Wallpaper Engine 未能启动壁纸的独立播放窗口。".into());
    }
    let handle = preview_handle(&capture.window)?;
    let input = format!(
        "gfxcapture=hwnd={handle}:capture_cursor=0:max_framerate=30,hwdownload,format=bgra"
    );
    let progress = capture.output.with_extension("progress");
    let mut recorder = quiet(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-n", "-progress"])
        .arg(&progress)
        .args(["-f", "lavfi", "-i"])
        .arg(input)
        .args(["-ss", RECORD_START_SECONDS, "-t", RECORD_SECONDS, "-vf",
            "fps=30,scale=1920:1080:force_original_aspect_ratio=increase:force_divisible_by=2,crop=1920:1080,setsar=1",
            "-an", "-c:v", "libx264", "-preset", "fast", "-crf", "18", "-pix_fmt", "yuv420p", "-movflags", "+faststart"])
        .arg(&capture.output)
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(io_error)?;
    if !wait_for(&mut recorder, Duration::from_secs(50))
        .map_err(io_error)?
        .success()
    {
        return Err("无法录制场景窗口。请保持 Wallpaper Engine 播放窗口开启后重试。".into());
    }
    let status = fs::read_to_string(progress).map_err(io_error)?;
    let frames = status.lines().rev().find_map(|line| {
        line.strip_prefix("frame=")
            .and_then(|count| count.trim().parse::<u16>().ok())
    });
    if frames != Some(600) {
        return Err(
            "场景录像未录满 20 秒。请保持播放窗口开启，并关闭 Wallpaper Engine 的暂停状态后重试。"
                .into(),
        );
    }
    import_wallpaper(app, &capture.output)
}

#[tauri::command]
pub async fn import_workshop_wallpaper(app: AppHandle, item: String) -> Result<Media, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = IMPORT
            .try_lock()
            .map_err(|_| "正在导入另一个创意工坊壁纸。")?;
        let id = workshop_id(&item)?;
        let libraries = steam_libraries()?;
        let folder = libraries
            .iter()
            .map(|library| {
                library
                    .join("steamapps/workshop/content")
                    .join(WORKSHOP_APP)
                    .join(id.to_string())
            })
            .find(|folder| folder.join("project.json").is_file())
            .ok_or_else(|| {
                format!("本机未找到作品 {id}。请在 Steam 创意工坊订阅并等待下载完成。")
            })?;
        let project_file = safe_file(&folder, "project.json")?;
        let text = fs::read_to_string(&project_file).map_err(io_error)?;
        let project: WorkshopProject = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|error| format!("壁纸项目配置无法读取：{error}"))?;
        match project.kind.to_ascii_lowercase().as_str() {
            "video" => {
                let file = safe_file(&folder, &project.file)?;
                if !file.extension().is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("mp4") || ext.eq_ignore_ascii_case("webm")
                }) {
                    return Err("创意工坊视频壁纸需要 MP4 或 WebM 文件。".into());
                }
                import_wallpaper(&app, &file)
            }
            "scene" => {
                safe_file(&folder, "scene.pkg")?;
                capture_scene(&app, &libraries, &project_file)
            }
            _ => Err("请使用创意工坊中类型为「视频」或「场景」的壁纸。".into()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}
