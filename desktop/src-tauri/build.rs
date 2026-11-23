fn main() {
    // 图标生成命令只更新资源，显式跟踪图标目录以便重新嵌入 Windows 图标。
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
