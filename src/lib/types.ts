export type ProjectKind = 'process' | 'powershell' | 'docker' | 'android';
export type ProjectPhase = 'setup' | 'stopped' | 'starting' | 'ready' | 'external' | 'stopping' | 'error' | 'unavailable';
export interface Profile {
  id: string;
  name: string;
  subtitle: string;
  kind: ProjectKind;
  path: string;
  url?: string;
  accent: string;
  cover: string;
  favorite: boolean;
  tags: string[];
  config: Record<string, unknown>;
}
export interface ProjectStatus {
  id: string;
  phase: ProjectPhase;
  owned: boolean;
  message?: string;
  pid?: number;
  serial?: string;
  startedAt?: string;
}
export interface LogEntry {
  timestamp: string;
  level: 'info' | 'warn' | 'error';
  message: string;
}
export interface Settings {
  closeToTray: boolean;
  autoOpenBrowser: boolean;
  theme: 'light';
  liveActionsEnabled: boolean;
}
export interface AndroidDevice { serial: string; state: string; model?: string; avd?: string }
export interface QuitSummary { managed: ProjectStatus[]; retained: string[]; failed: string[] }
export type DiagnosticState = 'checking' | 'unknown' | 'ready' | 'error' | 'notApplicable';
export interface DiagnosticCheck {
  id: 'dependencies' | 'process' | 'http' | 'port';
  label: string;
  state: DiagnosticState;
  message: string;
  value?: string;
  httpStatus?: number;
}
export interface ProjectDiagnostics {
  id: string;
  checkedAt: string;
  checks: DiagnosticCheck[];
  technologies: string[];
  canStop: boolean;
  canRestart: boolean;
  stopReason?: string;
  restartReason?: string;
}
export interface ProfileValidation { complete: boolean; issues: string[] }
