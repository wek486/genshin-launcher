use serde::Serialize;
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

#[cfg(windows)]
use std::os::windows::io::OwnedHandle as Process;
#[cfg(not(windows))]
use std::process::Child as Process;

#[derive(Default)]
pub struct GameProcess(Mutex<LaunchState>);

#[derive(Default)]
enum LaunchState {
    #[default]
    Idle,
    AwaitingConsent,
    Running(Process),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameStatus {
    executable_path: Option<PathBuf>,
    running: bool,
    available: bool,
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|dir| dir.join("genshin-path.json"))
        .map_err(|error| error.to_string())
}

fn read_path(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    match fs::read(config_path(app)?) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|_| "游戏路径配置无法读取，请重新选择。".into())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("无法读取游戏路径：{error}")),
    }
}

fn executable_exists(path: &Path) -> bool {
    path.is_absolute()
        && path.is_file()
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
}

#[cfg(windows)]
fn process_running(process: &mut Process) -> io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::WaitForSingleObject,
    };

    match unsafe { WaitForSingleObject(process.as_raw_handle(), 0) } {
        WAIT_OBJECT_0 => Ok(false),
        WAIT_TIMEOUT => Ok(true),
        _ => Err(io::Error::last_os_error()),
    }
}

#[cfg(not(windows))]
fn process_running(process: &mut Process) -> io::Result<bool> {
    process.try_wait().map(|status| status.is_none())
}

fn is_running(state: &mut LaunchState) -> Result<bool, String> {
    let LaunchState::Running(process) = state else {
        return Ok(false);
    };
    if process_running(process).map_err(|error| format!("无法读取游戏进程状态：{error}"))?
    {
        return Ok(true);
    }
    *state = LaunchState::Idle;
    Ok(false)
}

#[tauri::command]
pub fn game_status(app: AppHandle, process: State<GameProcess>) -> Result<GameStatus, String> {
    let path = read_path(&app)?;
    let mut state = process.0.lock().map_err(|error| error.to_string())?;
    Ok(GameStatus {
        available: path.as_deref().is_some_and(executable_exists),
        executable_path: path,
        running: is_running(&mut state)?,
    })
}

/// 由本机文件选择器选择入口，并原子保存路径。
#[tauri::command]
pub async fn choose_game_path(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<PathBuf>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(selection) = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("选择原神游戏程序")
            .add_filter("Windows 程序", &["exe"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let executable = selection.into_path().map_err(|error| error.to_string())?;
        if !executable_exists(&executable) {
            return Err("请选择已安装的原神程序或官方启动器 .exe。".into());
        }
        let path = config_path(&app)?;
        let directory = path.parent().ok_or("无法找到应用数据目录。")?;
        fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        let pending = directory.join(format!("genshin-path-{}.pending", Uuid::new_v4()));
        let data = serde_json::to_vec(&executable).map_err(|error| error.to_string())?;
        if let Err(error) = fs::write(&pending, data).and_then(|_| fs::rename(&pending, &path)) {
            let _ = fs::remove_file(&pending);
            return Err(format!("无法保存游戏路径：{error}"));
        }
        Ok(Some(executable))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(windows)]
fn launch_program(
    executable: &Path,
    directory: &Path,
    owner: usize,
) -> Result<Option<Process>, String> {
    use std::os::windows::{ffi::OsStrExt, io::FromRawHandle};
    use windows_sys::Win32::{
        Foundation::ERROR_CANCELLED,
        System::Com::{
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
        },
        UI::{
            Shell::{
                SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
                ShellExecuteExW,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    };

    let com = unsafe {
        CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        )
    };
    if com < 0 {
        return Err(format!("无法初始化 Windows 启动服务（{com:#010x}）。"));
    }

    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let file: Vec<u16> = executable.as_os_str().encode_wide().chain([0]).collect();
    let workdir: Vec<u16> = directory.as_os_str().encode_wide().chain([0]).collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        hwnd: owner as _,
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpDirectory: workdir.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };

    let success = unsafe { ShellExecuteExW(&mut info) } != 0;
    let error = (!success).then(io::Error::last_os_error);
    unsafe { CoUninitialize() };
    if let Some(error) = error {
        if error.raw_os_error() == Some(ERROR_CANCELLED as i32) {
            return Err("已取消管理员授权，游戏未启动。".into());
        }
        return Err(format!("无法以管理员身份启动游戏：{error}"));
    }
    Ok((!info.hProcess.is_null()).then(|| unsafe { Process::from_raw_handle(info.hProcess) }))
}

#[cfg(not(windows))]
fn launch_program(
    executable: &Path,
    directory: &Path,
    _owner: usize,
) -> Result<Option<Process>, String> {
    std::process::Command::new(executable)
        .current_dir(directory)
        .spawn()
        .map(Some)
        .map_err(|error| format!("无法启动游戏程序：{error}"))
}

/// Windows 通过系统 UAC 授权启动已保存的程序。
#[tauri::command]
pub async fn launch_game(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    #[cfg(windows)]
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    #[cfg(not(windows))]
    let owner = {
        drop(window);
        0
    };
    tauri::async_runtime::spawn_blocking(move || {
        let executable = read_path(&app)?
            .filter(|path| executable_exists(path))
            .ok_or("找不到游戏程序，请在设置中重新选择 .exe。")?;
        let directory = executable.parent().ok_or("找不到游戏安装目录。")?;
        let process = app.state::<GameProcess>();
        {
            let mut state = process.0.lock().map_err(|error| error.to_string())?;
            if matches!(*state, LaunchState::AwaitingConsent) {
                return Err("正在等待 Windows 管理员授权。".into());
            }
            if is_running(&mut state)? {
                return Err("游戏程序已经在运行。".into());
            }
            *state = LaunchState::AwaitingConsent;
        }

        let outcome = launch_program(&executable, directory, owner);
        let mut state = process.0.lock().map_err(|error| error.to_string())?;
        *state = LaunchState::Idle;
        if let Some(handle) = outcome? {
            *state = LaunchState::Running(handle);
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}
