export type ServicePhase = "starting" | "ready" | "failed";

export interface ServiceStatus {
  phase: ServicePhase;
  message: string;
  logs: string[];
  pid: number | null;
}
