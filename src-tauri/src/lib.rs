use serde::Serialize;
use std::{
    collections::VecDeque,
    env,
    ffi::OsString,
    io::{BufRead, BufReader},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tauri::{AppHandle, Manager, RunEvent};

const DSH_PACKAGE: &str = "@deepseek-ai/dsh@0.1.0-rc.6";
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
                    logs: vec![format!("准备启动 {DSH_PACKAGE} web")],
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
                logs: vec![format!("准备启动 {DSH_PACKAGE} web --port {DSH_PORT}")],
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

        let npx_path = match resolve_npx() {
            Some(path) => path,
            None => {
                self.update_status(
                    generation,
                    ServicePhase::Failed,
                    "未找到 npx，请先安装 Node.js",
                );
                self.append_log(
                    generation,
                    "已检查 PATH、/opt/homebrew/bin 和 /usr/local/bin，均未发现 npx",
                );
                return;
            }
        };

        self.append_log(generation, format!("使用 npx：{}", npx_path.display()));
        self.update_status(generation, ServicePhase::Starting, "正在启动 dsh web");

        let mut command = Command::new(&npx_path);
        command
            .args([
                "--yes",
                DSH_PACKAGE,
                "web",
                "--host",
                DSH_HOST,
                "--port",
                &DSH_PORT.to_string(),
            ])
            .current_dir(default_workspace())
            .env("PATH", augmented_path(&npx_path))
            .env("npm_config_yes", "true")
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
        let pid = child.id();

        {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if self.generation.load(Ordering::Acquire) != generation {
                drop(inner);
                stop_child(child);
                return;
            }
            inner.status.pid = Some(pid);
            inner.child = Some(child);
        }

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

fn resolve_npx() -> Option<PathBuf> {
    if let Some(configured) = env::var_os("DSH_NPX_PATH") {
        if let Some(path) = normalize_executable(PathBuf::from(configured)) {
            return Some(path);
        }
    }

    let executable_names: &[&str] = if cfg!(windows) {
        &["npx.cmd", "npx.exe", "npx"]
    } else {
        &["npx"]
    };

    let mut directories: Vec<PathBuf> = env::var_os("PATH")
        .map(|value| env::split_paths(&value).collect())
        .unwrap_or_default();

    if cfg!(target_os = "macos") {
        directories.extend([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
        ]);
    }

    for directory in directories {
        for executable_name in executable_names {
            let candidate = directory.join(executable_name);
            if let Some(path) = normalize_executable(candidate) {
                return Some(path);
            }
        }
    }

    None
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

fn augmented_path(npx_path: &Path) -> OsString {
    let mut directories = VecDeque::new();
    if let Some(parent) = npx_path.parent() {
        directories.push_back(parent.to_path_buf());
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
    fn configured_npx_path_must_point_to_a_file() {
        let missing = PathBuf::from("/definitely/missing/npx");
        assert!(!is_executable_file(&missing));
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
