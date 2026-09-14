//! Windows 原生「另存为」对话框（Common Item Dialog 的 `IFileSaveDialog`）。
//!
//! 用途：网页发起下载时，让用户按**系统默认的保存交互**选择落盘位置。用的是 Explorer 同一个
//! 系统对话框，因此自带「同名覆盖确认」「新建文件夹」「侧边栏快速访问」「最近位置记忆」
//! （Common Item Dialog 会按可执行文件把上次访问的目录记进注册表）等行为，无需自己实现。
//!
//! ⚠️ 调用契约：必须在**主线程**上、且在 WebView2 的 `DownloadStarting` 回调里**同步**调用。
//! `IFileSaveDialog::Show` 会在当前线程上跑一个**模态消息循环**：
//!   - owner 窗口与本调用同一线程，所以对话框是**真模态**（owner 被系统禁用），
//!     用户不会绕过去点网页；
//!   - 消息循环照常派发 Tauri / WebView2 的消息，界面不会假死。
//!
//! 反过来，「后台线程弹框 + `run_on_main_thread` 回填结果」这条路走不通：
//!   1) owner 窗口与对话框跨线程 → 失去模态语义（系统不认跨线程 owner 的模态）；
//!   2) `ICoreWebView2DownloadStartingEventArgs` / `ICoreWebView2Deferral` 这类 COM 接口指针
//!      在 windows-rs 里**不是 Send**（`IUnknown(NonNull<c_void>)` 没有 `unsafe impl Send`），
//!      想送回主线程必须自己包 `unsafe impl Send` 的壳 —— 不值得为此引入跨线程 COM 的隐患。

use std::path::{Path, PathBuf};

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileSaveDialog, IFileSaveDialog, IShellItem, SHCreateItemFromParsingName, FOS_FORCEFILESYSTEM,
    FOS_NOREADONLYRETURN, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, SIGDN_FILESYSPATH,
};

/// 用户在「另存为」对话框里的选择结果。
pub(crate) enum SaveChoice {
    /// 用户确认了路径 —— 调用方应把下载文件落到这里。
    Save(PathBuf),
    /// 用户点了「取消」 —— 调用方应取消这次下载（而不是静默存到别处）。
    Cancelled,
    /// 对话框压根没起来（系统过老 / Shell 组件不可用） —— 调用方保持平台默认行为。
    Unavailable,
}

/// 弹系统「另存为」对话框，返回用户的选择（**阻塞到用户关掉对话框为止**）。
///
/// - `owner`：宿主窗口句柄；`None` 时不设 owner（对话框仍能显示，但不模态）。
/// - `suggested_name`：默认文件名（一般来自 `Content-Disposition` 或 URL 末段）。
///   调用方需保证非空 —— 空名字会让输入框空白，比「download」更难用。
/// - `default_dir`：打开时定位到的目录（一般是系统下载目录）；`None` 则用系统记忆的位置。
pub(crate) fn ask_save_path(
    owner: Option<HWND>,
    suggested_name: &str,
    default_dir: Option<&Path>,
) -> SaveChoice {
    unsafe {
        let dialog: IFileSaveDialog =
            match CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER) {
                Ok(d) => d,
                // 极老系统或 Shell 组件缺失：不阻断下载，交回调用方走平台默认行为。
                Err(_) => return SaveChoice::Unavailable,
            };

        // 默认文件名（站点给的名字）。
        let _ = dialog.SetFileName(&HSTRING::from(suggested_name));

        // 扩展名过滤：按建议文件名的后缀收敛，用户改成别的后缀时系统会给出提醒。
        // ⚠️ `COMDLG_FILTERSPEC` 里只存裸指针、**不持有**字符串所有权，两个 `Vec<u16>`
        // 必须活到 `SetFileTypes` 返回之后（放在同一个 if 块里即可）。
        let ext = Path::new(suggested_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !ext.is_empty() {
            let label: Vec<u16> = format!("{ext} 文件 (*.{ext})")
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let spec: Vec<u16> = format!("*.{ext}")
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let filters = [COMDLG_FILTERSPEC {
                pszName: PCWSTR(label.as_ptr()),
                pszSpec: PCWSTR(spec.as_ptr()),
            }];
            let _ = dialog.SetFileTypes(&filters);
            let _ = dialog.SetDefaultExtension(&HSTRING::from(ext));
        }

        // 默认目录 = 系统下载目录（与「浏览器另存为」的默认落点一致）。
        // 解析失败（目录不存在、Shell 异常）就跳过，用系统记忆的位置，不影响可用性。
        if let Some(dir) = default_dir {
            let dir_h = HSTRING::from(dir.to_string_lossy().as_ref());
            if let Ok(folder) = SHCreateItemFromParsingName::<_, _, IShellItem>(&dir_h, None) {
                let _ = dialog.SetDefaultFolder(&folder);
            }
        }

        // 选项（都是「系统默认保存交互」的组成部分）：
        // 只接受文件系统路径 / 目录必须存在 / 同名时弹系统覆盖确认 / 不允许落到只读文件。
        let _ = dialog.SetOptions(
            FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_OVERWRITEPROMPT | FOS_NOREADONLYRETURN,
        );

        // 模态显示。用户点「取消」（或关掉窗口）时返回 ERROR_CANCELLED，属正常路径。
        if dialog.Show(owner).is_err() {
            return SaveChoice::Cancelled;
        }

        // 取结果：`GetDisplayName` 返回的是 CoTaskMem 分配的宽字符串，读完必须释放。
        let Ok(item) = dialog.GetResult() else {
            return SaveChoice::Cancelled;
        };
        let Ok(path_w) = item.GetDisplayName(SIGDN_FILESYSPATH) else {
            return SaveChoice::Cancelled;
        };
        let path = path_w
            .to_string()
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        if !path_w.is_null() {
            CoTaskMemFree(Some(path_w.0 as *const core::ffi::c_void));
        }

        match path {
            Some(p) => SaveChoice::Save(p),
            None => SaveChoice::Cancelled,
        }
    }
}
