use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    env,
    ffi::OsString,
    fs,
    io::{BufRead, BufReader},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tauri::{AppHandle, Manager, RunEvent};

const DSH_PACKAGE: &str = "@deepseek-ai/dsh@latest";
const DSH_INSTALL_MARKER: &str = ".dsh-desktop-installed";
const DSH_HOST: &str = "127.0.0.1";
const DSH_PORT: u16 = 3080;
const STARTUP_ATTEMPTS: usize = 120;
const LOG_LIMIT: usize = 80;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
enum ServicePhase {
    Starting,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceStatus {
    phase: ServicePhase,
    message: String,
    logs: Vec<String>,
    pid: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct DshPackageMetadata {
    name: String,
    version: String,
}

#[derive(Debug)]
struct CachedDsh {
    path: PathBuf,
    version: String,
    modified: std::time::SystemTime,
}

struct ServiceInner {
    child: Option<Child>,
    status: ServiceStatus,
}

struct ServiceManager {
    inner: Mutex<ServiceInner>,
    generation: AtomicU64,
    shutting_down: AtomicBool,
}

impl ServiceManager {
    fn new() -> Self {
        Self {
            inner: Mutex::new(ServiceInner {
                child: None,
                status: ServiceStatus {
                    phase: ServicePhase::Starting,
                    message: "正在准备本地运行环境".to_owned(),
                    logs: vec!["准备检查并启动 dsh web".to_owned()],
                    pid: None,
                },
            }),
            generation: AtomicU64::new(0),
            shutting_down: AtomicBool::new(false),
        }
    }

    fn snapshot(&self) -> ServiceStatus {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .status
            .clone()
    }

    fn update_status(&self, generation: u64, phase: ServicePhase, message: impl Into<String>) {
        if self.generation.load(Ordering::Acquire) != generation {
            return;
        }

        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.status.phase = phase;
        inner.status.message = message.into();
    }

    fn append_log(&self, generation: u64, line: impl Into<String>) {
        if self.generation.load(Ordering::Acquire) != generation {
            return;
        }

        let line = line.into();
        if line.trim().is_empty() {
            return;
        }

        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.status.logs.push(line);
        let overflow = inner.status.logs.len().saturating_sub(LOG_LIMIT);
        if overflow > 0 {
            inner.status.logs.drain(0..overflow);
        }
    }

    fn take_child(&self) -> Option<Child> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.status.pid = None;
        inner.child.take()
    }

    fn track_child(&self, generation: u64, child: Child) -> Result<u32, Child> {
        if self.generation.load(Ordering::Acquire) != generation {
            return Err(child);
        }

        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.generation.load(Ordering::Acquire) != generation || inner.child.is_some() {
            return Err(child);
        }

        let pid = child.id();
        inner.status.pid = Some(pid);
        inner.child = Some(child);
        Ok(pid)
    }

    fn wait_for_tracked_child(&self, generation: u64) -> Result<ExitStatus, String> {
        loop {
            if self.generation.load(Ordering::Acquire) != generation
                || self.shutting_down.load(Ordering::Acquire)
            {
                return Err("操作已取消".to_owned());
            }

            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(child) = inner.child.as_mut() else {
                return Err("安装进程意外丢失".to_owned());
            };

            match child.try_wait() {
                Ok(Some(status)) => {
                    inner.child = None;
                    inner.status.pid = None;
                    return Ok(status);
                }
                Ok(None) => {}
                Err(error) => {
                    inner.child = None;
                    inner.status.pid = None;
                    return Err(format!("无法读取安装进程状态：{error}"));
                }
            }
            drop(inner);
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn launch(self: &Arc<Self>, app: AppHandle) {
        self.shutting_down.store(false, Ordering::Release);
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;

        if let Some(child) = self.take_child() {
            stop_child(child);
        }

        {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            inner.status = ServiceStatus {
                phase: ServicePhase::Starting,
                message: "正在检查本地端口".to_owned(),
                logs: vec![format!("准备启动 dsh web --port {DSH_PORT}")],
                pid: None,
            };
        }

        let manager = Arc::clone(self);
        thread::spawn(move || manager.launch_in_background(app, generation));
    }

    fn launch_in_background(self: Arc<Self>, app: AppHandle, generation: u64) {
        if service_is_reachable() {
            self.update_status(
                generation,
                ServicePhase::Failed,
                format!("端口 {DSH_PORT} 已被其他进程占用"),
            );
            self.append_log(
                generation,
                format!("无法启动：{DSH_HOST}:{DSH_PORT} 已有服务监听"),
            );
            return;
        }

        let dsh_path = match self.ensure_dsh(&app, generation) {
            Some(path) => path,
            None => {
                return;
            }
        };

        self.append_log(generation, format!("使用 dsh：{}", dsh_path.display()));
        self.update_status(generation, ServicePhase::Starting, "正在启动 dsh web");

        let mut command = Command::new(&dsh_path);
        command
            .args(["web", "--host", DSH_HOST, "--port", &DSH_PORT.to_string()])
            .current_dir(default_workspace())
            .env("PATH", augmented_path(&dsh_path))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        configure_process_group(&mut command);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                self.update_status(generation, ServicePhase::Failed, "无法创建 dsh 进程");
                self.append_log(generation, format!("启动失败：{error}"));
                return;
            }
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let pid = match self.track_child(generation, child) {
            Ok(pid) => pid,
            Err(child) => {
                stop_child(child);
                return;
            }
        };

        self.append_log(generation, format!("dsh 进程已启动，PID {pid}"));

        if let Some(stdout) = stdout {
            pipe_logs(Arc::clone(&self), generation, stdout, "dsh");
        }
        if let Some(stderr) = stderr {
            pipe_logs(Arc::clone(&self), generation, stderr, "dsh");
        }

        for _ in 0..STARTUP_ATTEMPTS {
            if self.generation.load(Ordering::Acquire) != generation
                || self.shutting_down.load(Ordering::Acquire)
            {
                return;
            }

            if let Some(exit_message) = self.child_exit_message() {
                self.update_status(generation, ServicePhase::Failed, exit_message.clone());
                self.append_log(generation, exit_message);
                return;
            }

            if service_is_reachable() {
                self.update_status(generation, ServicePhase::Ready, "本地服务已就绪，正在打开");
                self.append_log(
                    generation,
                    format!("服务已就绪：http://{DSH_HOST}:{DSH_PORT}"),
                );
                navigate_to_service(&app);
                return;
            }

            thread::sleep(Duration::from_millis(500));
        }

        self.update_status(
            generation,
            ServicePhase::Failed,
            "启动超时，请检查网络或 npm 配置",
        );
        self.append_log(generation, "等待本地服务超过 60 秒");
        if let Some(child) = self.take_child() {
            stop_child(child);
        }
    }

    fn ensure_dsh(self: &Arc<Self>, app: &AppHandle, generation: u64) -> Option<PathBuf> {
        if let Some(path) = configured_executable("DSH_EXECUTABLE_PATH") {
            self.append_log(generation, format!("检测到指定的 dsh：{}", path.display()));
            return Some(path);
        }

        if let Some(path) = resolve_system_dsh() {
            self.append_log(generation, format!("检测到系统 dsh：{}", path.display()));
            return Some(path);
        }

        let runtime_dir = match managed_runtime_dir(app) {
            Ok(path) => path,
            Err(error) => {
                self.update_status(generation, ServicePhase::Failed, "无法确定 dsh 安装目录");
                self.append_log(generation, error);
                return None;
            }
        };

        if let Some(path) = resolve_managed_dsh(&runtime_dir) {
            self.append_log(generation, format!("使用已安装的 dsh：{}", path.display()));
            return Some(path);
        }

        if let Some(cached) = resolve_npx_cached_dsh() {
            self.append_log(
                generation,
                format!(
                    "检测到 npx 缓存中的 dsh {}：{}",
                    cached.version,
                    cached.path.display()
                ),
            );
            return Some(cached.path);
        }

        match self.install_managed_dsh(generation, &runtime_dir) {
            Ok(path) => Some(path),
            Err(error) => {
                self.update_status(generation, ServicePhase::Failed, "dsh 自动安装失败");
                self.append_log(generation, error);
                None
            }
        }
    }

    fn install_managed_dsh(
        self: &Arc<Self>,
        generation: u64,
        runtime_dir: &Path,
    ) -> Result<PathBuf, String> {
        let npm_path = resolve_npm()
            .ok_or_else(|| "未找到 npm，请先安装包含 npm 的 Node.js 20 或更高版本".to_owned())?;
        let cache_dir = runtime_dir.join("npm-cache");

        fs::create_dir_all(runtime_dir)
            .map_err(|error| format!("无法创建 dsh 安装目录：{error}"))?;
        fs::create_dir_all(&cache_dir)
            .map_err(|error| format!("无法创建 npm 缓存目录：{error}"))?;

        self.update_status(
            generation,
            ServicePhase::Starting,
            "未检测到 dsh，正在安装最新版",
        );
        self.append_log(generation, format!("使用 npm：{}", npm_path.display()));
        self.append_log(
            generation,
            format!("正在安装 {DSH_PACKAGE}，首次运行可能需要一些时间"),
        );

        let mut command = Command::new(&npm_path);
        command
            .args(npm_install_arguments(runtime_dir, &cache_dir))
            .current_dir(runtime_dir)
            .env("PATH", augmented_path(&npm_path))
            .env("npm_config_update_notifier", "false")
            .env("npm_config_yes", "true")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        configure_process_group(&mut command);

        let mut child = command
            .spawn()
            .map_err(|error| format!("无法创建 npm 安装进程：{error}"))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let pid = match self.track_child(generation, child) {
            Ok(pid) => pid,
            Err(child) => {
                stop_child(child);
                return Err("dsh 安装已取消".to_owned());
            }
        };

        self.append_log(generation, format!("npm 安装进程已启动，PID {pid}"));
        if let Some(stdout) = stdout {
            pipe_logs(Arc::clone(self), generation, stdout, "npm");
        }
        if let Some(stderr) = stderr {
            pipe_logs(Arc::clone(self), generation, stderr, "npm");
        }

        let status = self.wait_for_tracked_child(generation)?;
        if !status.success() {
            return Err(format!("npm 安装失败：{status}"));
        }

        let dsh_path = normalize_executable(managed_dsh_path(runtime_dir))
            .ok_or_else(|| "npm 已完成，但安装目录中未找到 dsh 可执行文件".to_owned())?;
        fs::write(managed_install_marker(runtime_dir), DSH_PACKAGE)
            .map_err(|error| format!("无法写入 dsh 安装完成标记：{error}"))?;

        self.append_log(
            generation,
            format!("dsh 最新版安装完成：{}", dsh_path.display()),
        );
        Ok(dsh_path)
    }

    fn child_exit_message(&self) -> Option<String> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let child = inner.child.as_mut()?;

        match child.try_wait() {
            Ok(Some(status)) => {
                inner.child = None;
                inner.status.pid = None;
                Some(format!("dsh 进程已退出：{status}"))
            }
            Ok(None) => None,
            Err(error) => Some(format!("无法读取 dsh 进程状态：{error}")),
        }
    }

    fn shutdown(&self) {
        if self.shutting_down.swap(true, Ordering::AcqRel) {
            return;
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Some(child) = self.take_child() {
            stop_child(child);
        }
    }
}

#[tauri::command]
fn service_status(manager: tauri::State<'_, Arc<ServiceManager>>) -> ServiceStatus {
    manager.snapshot()
}

#[tauri::command]
fn restart_service(
    app: AppHandle,
    manager: tauri::State<'_, Arc<ServiceManager>>,
) -> ServiceStatus {
    let manager = Arc::clone(manager.inner());
    manager.launch(app);
    manager.snapshot()
}

fn pipe_logs<R>(manager: Arc<ServiceManager>, generation: u64, reader: R, source: &'static str)
where
    R: std::io::Read + Send + 'static,
{
    thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            match line {
                Ok(line) => manager.append_log(generation, format!("[{source}] {line}")),
                Err(error) => {
                    manager.append_log(generation, format!("[{source}] 日志读取失败：{error}"));
                    break;
                }
            }
        }
    });
}

fn service_is_reachable() -> bool {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DSH_PORT);
    TcpStream::connect_timeout(&address, Duration::from_millis(250)).is_ok()
}

fn resolve_system_dsh() -> Option<PathBuf> {
    find_executable_in_directories(&executable_search_directories(), dsh_executable_names())
}

fn resolve_npx_cached_dsh() -> Option<CachedDsh> {
    let cache_dir = npm_cache_directory()?;
    resolve_npx_cached_dsh_in(&cache_dir)
}

fn resolve_npx_cached_dsh_in(cache_dir: &Path) -> Option<CachedDsh> {
    let entries = fs::read_dir(cache_dir.join("_npx")).ok()?;
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| cached_dsh_from_npx_entry(&entry.path()))
        .max_by_key(|cached| cached.modified)
}

fn cached_dsh_from_npx_entry(entry_dir: &Path) -> Option<CachedDsh> {
    let package_path = entry_dir
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh")
        .join("package.json");
    let package_contents = fs::read_to_string(&package_path).ok()?;
    let package: DshPackageMetadata = serde_json::from_str(&package_contents).ok()?;
    if package.name != "@deepseek-ai/dsh" || package.version.trim().is_empty() {
        return None;
    }

    let executable_name = if cfg!(windows) { "dsh.cmd" } else { "dsh" };
    let path = normalize_executable(
        entry_dir
            .join("node_modules")
            .join(".bin")
            .join(executable_name),
    )?;
    let modified = package_path.metadata().ok()?.modified().ok()?;

    Some(CachedDsh {
        path,
        version: package.version,
        modified,
    })
}

fn npm_cache_directory() -> Option<PathBuf> {
    env::var_os("DSH_NPM_CACHE_PATH")
        .or_else(|| env::var_os("npm_config_cache"))
        .map(PathBuf::from)
        .or_else(default_npm_cache_directory)
        .filter(|path| path.is_absolute())
}

fn default_npm_cache_directory() -> Option<PathBuf> {
    if cfg!(windows) {
        env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("npm-cache"))
    } else {
        user_home_directory().map(|path| path.join(".npm"))
    }
}

fn resolve_npm() -> Option<PathBuf> {
    if let Some(path) = configured_executable("DSH_NPM_PATH") {
        return Some(path);
    }

    if let Some(configured_npx) = env::var_os("DSH_NPX_PATH") {
        let configured_npx = PathBuf::from(configured_npx);
        if let Some(parent) = configured_npx.parent() {
            if let Some(path) =
                find_executable_in_directories(&[parent.to_path_buf()], npm_executable_names())
            {
                return Some(path);
            }
        }
    }

    find_executable_in_directories(&executable_search_directories(), npm_executable_names())
}

fn resolve_node() -> Option<PathBuf> {
    configured_executable("DSH_NODE_PATH").or_else(|| {
        find_executable_in_directories(&executable_search_directories(), node_executable_names())
    })
}

fn configured_executable(variable_name: &str) -> Option<PathBuf> {
    env::var_os(variable_name).and_then(|value| normalize_executable(PathBuf::from(value)))
}

fn find_executable_in_directories(
    directories: &[PathBuf],
    executable_names: &[&str],
) -> Option<PathBuf> {
    directories.iter().find_map(|directory| {
        executable_names
            .iter()
            .find_map(|name| normalize_executable(directory.join(name)))
    })
}

fn executable_search_directories() -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = env::var_os("PATH")
        .map(|value| env::split_paths(&value).collect())
        .unwrap_or_default();

    if let Some(home) = user_home_directory() {
        directories.extend([home.join(".local/bin"), home.join(".npm-global/bin")]);
    }

    if cfg!(target_os = "macos") {
        directories.extend([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
        ]);
    }

    directories
}

fn dsh_executable_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["dsh.cmd", "dsh.exe", "dsh"]
    } else {
        &["dsh"]
    }
}

fn npm_executable_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["npm.cmd", "npm.exe", "npm"]
    } else {
        &["npm"]
    }
}

fn node_executable_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["node.exe", "node"]
    } else {
        &["node"]
    }
}

fn managed_runtime_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(configured) = env::var_os("DSH_RUNTIME_DIR") {
        return validate_runtime_dir(PathBuf::from(configured));
    }

    app.path()
        .app_local_data_dir()
        .map(|path| path.join("dsh-runtime"))
        .map_err(|error| format!("无法解析应用数据目录：{error}"))
}

fn validate_runtime_dir(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Err("DSH_RUNTIME_DIR 必须是绝对路径".to_owned())
    }
}

fn managed_dsh_path(runtime_dir: &Path) -> PathBuf {
    let executable_name = if cfg!(windows) { "dsh.cmd" } else { "dsh" };
    runtime_dir
        .join("node_modules")
        .join(".bin")
        .join(executable_name)
}

fn managed_install_marker(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(DSH_INSTALL_MARKER)
}

fn resolve_managed_dsh(runtime_dir: &Path) -> Option<PathBuf> {
    if !managed_install_marker(runtime_dir).is_file() {
        return None;
    }

    normalize_executable(managed_dsh_path(runtime_dir))
}

fn npm_install_arguments(runtime_dir: &Path, cache_dir: &Path) -> Vec<OsString> {
    vec![
        OsString::from("install"),
        OsString::from("--prefix"),
        runtime_dir.as_os_str().to_os_string(),
        OsString::from("--cache"),
        cache_dir.as_os_str().to_os_string(),
        OsString::from("--no-save"),
        OsString::from("--package-lock=false"),
        OsString::from("--no-audit"),
        OsString::from("--no-fund"),
        OsString::from(DSH_PACKAGE),
    ]
}

fn normalize_executable(path: PathBuf) -> Option<PathBuf> {
    let absolute_path = if path.is_absolute() {
        path
    } else {
        env::current_dir().ok()?.join(path)
    };

    if !is_executable_file(&absolute_path) {
        return None;
    }

    absolute_path.canonicalize().ok().or(Some(absolute_path))
}

fn is_executable_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|metadata| metadata.is_file())
}

fn augmented_path(executable_path: &Path) -> OsString {
    let mut directories = VecDeque::new();
    if let Some(parent) = executable_path.parent() {
        directories.push_back(parent.to_path_buf());
    }

    if let Some(node_path) = resolve_node() {
        if let Some(parent) = node_path.parent() {
            directories.push_back(parent.to_path_buf());
        }
    }

    if cfg!(target_os = "macos") {
        directories.extend([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
            PathBuf::from("/usr/sbin"),
            PathBuf::from("/sbin"),
        ]);
    }

    if let Some(current) = env::var_os("PATH") {
        directories.extend(env::split_paths(&current));
    }

    env::join_paths(directories).unwrap_or_else(|_| env::var_os("PATH").unwrap_or_default())
}

fn default_workspace() -> PathBuf {
    env::var_os("DSH_WORKSPACE")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .or_else(user_home_directory)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn user_home_directory() -> Option<PathBuf> {
    let variable_names = if cfg!(windows) {
        ["USERPROFILE", "HOME"]
    } else {
        ["HOME", "USERPROFILE"]
    };

    variable_names
        .into_iter()
        .filter_map(env::var_os)
        .map(PathBuf::from)
        .find(|path| path.is_dir())
}

fn navigate_to_service(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let url = format!("http://{DSH_HOST}:{DSH_PORT}");
    if let Ok(parsed) = url.parse() {
        let _ = window.navigate(parsed);
    }
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn stop_child(mut child: Child) {
    let process_group = -(child.id() as i32);
    unsafe {
        libc::kill(process_group, libc::SIGTERM);
    }

    for _ in 0..20 {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }

    unsafe {
        libc::kill(process_group, libc::SIGKILL);
    }
    let _ = child.wait();
}

#[cfg(windows)]
fn stop_child(mut child: Child) {
    let pid = child.id().to_string();
    let _ = Command::new("taskkill")
        .args(["/PID", &pid, "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.wait();
}

#[cfg(not(any(unix, windows)))]
fn stop_child(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub fn run() {
    let manager = Arc::new(ServiceManager::new());
    let managed = Arc::clone(&manager);
    let signal_manager = Arc::clone(&manager);

    ctrlc::set_handler(move || {
        signal_manager.shutdown();
        std::process::exit(0);
    })
    .expect("failed to register the process shutdown handler");

    tauri::Builder::default()
        .manage(managed)
        .invoke_handler(tauri::generate_handler![service_status, restart_service])
        .setup(move |app| {
            manager.launch(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build DSH Desktop")
        .run(|app_handle, event| match event {
            RunEvent::ExitRequested { .. } | RunEvent::Exit => {
                app_handle.state::<Arc<ServiceManager>>().shutdown();
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_executable_path_must_point_to_a_file() {
        let missing = PathBuf::from("/definitely/missing/dsh");
        assert!(!is_executable_file(&missing));
    }

    #[test]
    fn npm_install_arguments_use_latest_package_and_isolated_directories() {
        let runtime_dir = PathBuf::from("managed-runtime");
        let cache_dir = runtime_dir.join("npm-cache");

        let arguments = npm_install_arguments(&runtime_dir, &cache_dir);

        assert_eq!(arguments[0], "install");
        assert_eq!(arguments[2], runtime_dir.as_os_str());
        assert_eq!(arguments[4], cache_dir.as_os_str());
        assert!(arguments.contains(&OsString::from("--no-save")));
        assert!(arguments.contains(&OsString::from("--package-lock=false")));
        assert_eq!(arguments.last(), Some(&OsString::from(DSH_PACKAGE)));
    }

    #[test]
    fn managed_runtime_override_must_be_absolute() {
        let absolute = if cfg!(windows) {
            PathBuf::from(r"C:\dsh-desktop-runtime")
        } else {
            PathBuf::from("/dsh-desktop-runtime")
        };

        assert!(validate_runtime_dir(PathBuf::from("relative/runtime")).is_err());
        assert_eq!(validate_runtime_dir(absolute.clone()).unwrap(), absolute);
    }

    #[test]
    fn managed_dsh_requires_a_completed_install_marker() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let runtime_dir = env::temp_dir().join(format!(
            "dsh-desktop-managed-runtime-{}-{unique}",
            std::process::id()
        ));
        let dsh_path = managed_dsh_path(&runtime_dir);
        fs::create_dir_all(dsh_path.parent().unwrap()).unwrap();
        fs::write(&dsh_path, "test dsh executable").unwrap();

        assert!(resolve_managed_dsh(&runtime_dir).is_none());

        fs::write(managed_install_marker(&runtime_dir), DSH_PACKAGE).unwrap();
        let resolved = resolve_managed_dsh(&runtime_dir).expect("managed dsh should resolve");

        assert_eq!(resolved, dsh_path.canonicalize().unwrap());
        fs::remove_dir_all(runtime_dir).unwrap();
    }

    #[test]
    fn npx_cached_dsh_is_detected_from_a_valid_package() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cache_dir = env::temp_dir().join(format!(
            "dsh-desktop-npx-cache-{}-{unique}",
            std::process::id()
        ));
        let entry_dir = cache_dir.join("_npx").join("valid-entry");
        let package_dir = entry_dir
            .join("node_modules")
            .join("@deepseek-ai")
            .join("dsh");
        let executable_name = if cfg!(windows) { "dsh.cmd" } else { "dsh" };
        let executable = entry_dir
            .join("node_modules")
            .join(".bin")
            .join(executable_name);
        fs::create_dir_all(&package_dir).unwrap();
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(
            package_dir.join("package.json"),
            r#"{"name":"@deepseek-ai/dsh","version":"1.2.3"}"#,
        )
        .unwrap();
        fs::write(&executable, "test dsh executable").unwrap();

        let cached = resolve_npx_cached_dsh_in(&cache_dir).expect("cached dsh should resolve");

        assert_eq!(cached.version, "1.2.3");
        assert_eq!(cached.path, executable.canonicalize().unwrap());
        fs::remove_dir_all(cache_dir).unwrap();
    }

    #[test]
    fn npx_cache_entry_with_another_package_name_is_ignored() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cache_dir = env::temp_dir().join(format!(
            "dsh-desktop-invalid-npx-cache-{}-{unique}",
            std::process::id()
        ));
        let entry_dir = cache_dir.join("_npx").join("invalid-entry");
        let package_dir = entry_dir
            .join("node_modules")
            .join("@deepseek-ai")
            .join("dsh");
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(
            package_dir.join("package.json"),
            r#"{"name":"not-dsh","version":"1.2.3"}"#,
        )
        .unwrap();

        assert!(resolve_npx_cached_dsh_in(&cache_dir).is_none());
        fs::remove_dir_all(cache_dir).unwrap();
    }

    #[test]
    fn relative_executable_path_is_normalized_to_an_absolute_path() {
        let current_dir = env::current_dir().expect("current directory should be available");
        let current_exe = env::current_exe().expect("test executable should be available");
        let relative_exe = current_exe
            .strip_prefix(&current_dir)
            .expect("test executable should be inside the project directory")
            .to_path_buf();

        let normalized =
            normalize_executable(relative_exe).expect("relative executable should resolve");

        assert!(normalized.is_absolute());
        assert_eq!(normalized, current_exe.canonicalize().unwrap());
    }

    #[test]
    fn default_workspace_is_an_existing_directory() {
        assert!(default_workspace().is_dir());
    }
}
