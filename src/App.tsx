import {
  CircleAlert,
  LoaderCircle,
  RefreshCw,
  ShieldCheck,
  TerminalSquare,
} from "lucide-react";
import { useDshService } from "./hooks/useDshService";
import type { ServicePhase } from "./types/service";
import whaleLogo from "./assets/deepseek-whale.svg";
import styles from "./App.module.css";

interface PhaseContent {
  eyebrow: string;
  title: string;
  description: string;
}

const PHASE_CONTENT: Record<ServicePhase, PhaseContent> = {
  starting: {
    eyebrow: "本地服务启动中",
    title: "正在打开 DeepSeek Harness",
    description: "优先复用系统命令或 npx 缓存，未检测到时自动安装最新版。",
  },
  ready: {
    eyebrow: "服务已就绪",
    title: "正在进入 DeepSeek Harness",
    description: "桌面窗口即将连接本地 Web 服务。",
  },
  failed: {
    eyebrow: "启动遇到问题",
    title: "DeepSeek Harness 未能启动",
    description: "请查看运行记录，处理问题后重新尝试。",
  },
};

export function App(): React.ReactElement {
  const { status, retry } = useDshService();
  const content = PHASE_CONTENT[status.phase];
  const isFailed = status.phase === "failed";

  return (
    <main className={styles.shell}>
      <header className={styles.header}>
        <img className={styles.brandMark} src={whaleLogo} alt="" />
        <div>
          <p className={styles.productName}>DSH Desktop</p>
          <p className={styles.productDetail}>DeepSeek Harness</p>
        </div>
      </header>

      <section className={styles.content} aria-live="polite">
        <div className={styles.summary}>
          <div className={`${styles.statusIcon} ${styles[status.phase]}`}>
            {isFailed ? (
              <CircleAlert aria-hidden="true" />
            ) : (
              <LoaderCircle className={styles.spinner} aria-hidden="true" />
            )}
          </div>

          <p className={styles.eyebrow}>{content.eyebrow}</p>
          <h1>{content.title}</h1>
          <p className={styles.description}>{content.description}</p>

          <div className={styles.progressTrack} aria-hidden="true">
            <span
              className={`${styles.progressBar} ${isFailed ? styles.progressFailed : ""}`}
            />
          </div>

          <dl className={styles.facts}>
            <div>
              <dt>当前状态</dt>
              <dd>{status.message}</dd>
            </div>
            <div>
              <dt>本地地址</dt>
              <dd>127.0.0.1:3080</dd>
            </div>
            <div>
              <dt>进程</dt>
              <dd>{status.pid ? `PID ${status.pid}` : "等待启动"}</dd>
            </div>
          </dl>

          {isFailed ? (
            <button className={styles.retryButton} type="button" onClick={() => void retry()}>
              <RefreshCw size={17} aria-hidden="true" />
              重新启动
            </button>
          ) : null}
        </div>

        <aside className={styles.console} aria-label="启动日志">
          <div className={styles.consoleHeader}>
            <TerminalSquare size={17} aria-hidden="true" />
            <span>启动日志</span>
            <span className={styles.consoleLive}>LIVE</span>
          </div>
          <div className={styles.logLines}>
            {status.logs.length > 0 ? (
              status.logs.map((line, index) => (
                <p key={`${index}-${line}`}>
                  <span aria-hidden="true">$</span>
                  {line}
                </p>
              ))
            ) : (
              <p>
                <span aria-hidden="true">$</span>
                等待进程输出...
              </p>
            )}
          </div>
        </aside>
      </section>

      <footer className={styles.footer}>
        <ShieldCheck size={15} aria-hidden="true" />
        <span>服务仅监听本机地址，关闭窗口将同时停止服务</span>
      </footer>
    </main>
  );
}
