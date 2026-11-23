# 原神启动器本地接口

前端通过 Tauri IPC 调用下列 Rust 命令。成功时返回表中数据，失败时拒绝 Promise 并返回可显示的中文错误。

| 命令 | 参数 | 返回与说明 |
| --- | --- | --- |
| `game_status` | 无 | `{ executablePath: string \| null, available: boolean, running: boolean }`。读取已保存路径、文件可用性与启动器跟踪的进程状态。 |
| `choose_game_path` | 无 | 打开本机选择器，保存选定的 `.exe`，返回绝对路径。取消时返回 `null`。 |
| `launch_game` | 无 | Windows 通过系统 UAC 请求管理员权限，再从可执行文件所在目录启动程序。用户取消时返回错误提示，等待授权和程序运行期间拒绝重复启动。 |
| `get_media` | `slot: 'icon' \| 'wallpaper' \| 'music'` | 返回已导入的媒体描述，未选择本地媒体时返回 `null`。 |
| `choose_media` | 同上 | 打开本机选择器。确认后导入并立即应用，返回媒体描述。取消时返回 `null`。 |
| `reset_media` | 同上 | 恢复默认外观或移除背景音乐，成功返回空值。 |
| `import_workshop_wallpaper` | `item: string`，创意工坊链接或作品 ID | 读取本机 Steam 已下载的壁纸。MP4、WebM 视频直接导入，场景壁纸跳过前 6 秒后录制 20 秒的 1080p、30 帧静音 MP4。 |

媒体描述包含 `path: string`（应用数据目录中的副本）和 `kind: 'image' | 'video' | 'audio'`。前端通过 `convertFileSrc` 将路径转换为资源 URL。音乐可通过 `musicApi.get()`、`musicApi.choose()` 和 `musicApi.remove()` 调用上述接口。
