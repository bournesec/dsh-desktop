import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { ServiceStatus } from "../types/service";

const INITIAL_STATUS: ServiceStatus = {
  phase: "starting",
  message: "正在准备本地运行环境",
  logs: ["等待桌面端连接 Rust 服务管理器..."],
  pid: null,
};

const isTauriRuntime = (): boolean => "__TAURI_INTERNALS__" in window;

const getPreviewStatus = (): ServiceStatus => {
  const showFailure =
    import.meta.env.DEV &&
    new URLSearchParams(window.location.search).get("preview") === "failed";

  if (showFailure) {
    return {
      phase: "failed",
      message: "端口 3080 已被其他进程占用",
      logs: [
        "无法启动：127.0.0.1:3080 已有服务监听",
        "关闭占用端口的进程后可重新启动。",
      ],
      pid: null,
    };
  }

  return {
    phase: "starting",
    message: "浏览器预览模式",
    logs: [
      "启动界面预览已就绪。",
      "在 Tauri 桌面进程中运行时，将自动启动 DeepSeek Harness。",
    ],
    pid: null,
  };
};

export interface UseDshServiceResult {
  status: ServiceStatus;
  retry: () => Promise<void>;
}

export function useDshService(): UseDshServiceResult {
  const [status, setStatus] = useState<ServiceStatus>(INITIAL_STATUS);

  const fetchStatus = useCallback(async (): Promise<void> => {
    if (!isTauriRuntime()) {
      setStatus(getPreviewStatus());
      return;
    }

    try {
      setStatus(await invoke<ServiceStatus>("service_status"));
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      setStatus({
        phase: "failed",
        message: "无法读取本地服务状态",
        logs: [message],
        pid: null,
      });
    }
  }, []);

  const retry = useCallback(async (): Promise<void> => {
    if (!isTauriRuntime()) {
      window.history.replaceState({}, "", window.location.pathname);
      await fetchStatus();
      return;
    }

    setStatus((current) => ({
      ...current,
      phase: "starting",
      message: "正在重新启动 DeepSeek Harness",
    }));

    try {
      setStatus(await invoke<ServiceStatus>("restart_service"));
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      setStatus({
        phase: "failed",
        message: "重新启动失败",
        logs: [message],
        pid: null,
      });
    }
  }, [fetchStatus]);

  useEffect(() => {
    void fetchStatus();
    const timer = window.setInterval(() => {
      void fetchStatus();
    }, 500);

    return () => window.clearInterval(timer);
  }, [fetchStatus]);

  return { status, retry };
}
