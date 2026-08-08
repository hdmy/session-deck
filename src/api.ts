import { invoke } from '@tauri-apps/api/core';
import type {
  AppStatus,
  ClaudeSettings,
  ClaudeSettingsUpdate,
  ContinuationStatus,
  IndexDiagnostics,
  Project,
  ResumePreview,
  ScanReport,
  ScanSettings,
  ScanSettingsUpdate,
  ScanTrigger,
  SearchHit,
  SessionDetail,
  SessionSummary,
  SourceRootActivationReport,
} from './types';

export const api = {
  scan: (trigger: ScanTrigger = 'manual') => invoke<ScanReport>('scan', { args: { trigger } }),
  projects: () => invoke<Project[]>('list_projects'),
  session: (sessionId: string) => invoke<SessionDetail>('get_session', { sessionId }),
  getSessionBranch: (sessionId: string, branchId: string) =>
    invoke<SessionDetail>('get_session_branch', { args: { sessionId, branchId } }),
  search: (query: string) => invoke<SearchHit[]>('global_search', { query }),
  listHiddenSessions: () => invoke<SessionSummary[]>('list_hidden_sessions'),
  setSessionHidden: (sessionId: string, hidden: boolean) =>
    invoke<SessionSummary>('set_session_hidden', { args: { sessionId, hidden } }),
  renameSession: (sessionId: string, title: string | null) =>
    invoke<SessionSummary>('rename_session', { args: { sessionId, title } }),
  setSessionPinned: (sessionId: string, pinned: boolean) =>
    invoke<SessionSummary>('set_session_pinned', { args: { sessionId, pinned } }),
  touchSession: (sessionId: string) =>
    invoke<SessionSummary>('touch_session', { args: { sessionId } }),
  status: () => invoke<AppStatus>('status'),
  getClaudeSettings: () => invoke<ClaudeSettings>('get_claude_settings'),
  updateClaudeSettings: (update: ClaudeSettingsUpdate) => invoke<ClaudeSettings>('update_claude_settings', { update }),
  getScanSettings: () => invoke<ScanSettings>('get_scan_settings'),
  updateScanSettings: (update: ScanSettingsUpdate) => invoke<ScanSettings>('update_scan_settings', {
    update: { scanIntervalSeconds: update.scan_interval_seconds },
  }),
  activateClaudeSourceRoot: (sourceRoot: string | null, replaceActiveIndexAcknowledged: boolean) =>
    invoke<SourceRootActivationReport>('activate_claude_source_root', {
      args: { sourceRoot, replaceActiveIndexAcknowledged },
    }),
  getIndexDiagnostics: () => invoke<IndexDiagnostics>('get_index_diagnostics'),
  setProjectAlias: (workspaceId: string, alias: string | null) =>
    invoke<Project[]>('set_project_alias', { args: { workspaceId, alias } }),
  resumePreflight: (sessionId: string) => invoke<ResumePreview>('resume_preflight', { args: { sessionId } }),
  startContinuation: (sessionId: string, rows?: number, cols?: number) => invoke<ContinuationStatus>('start_continuation', { args: { sessionId, rows, cols } }),
  startForkContinuation: (sessionId: string, rows?: number, cols?: number) => invoke<ContinuationStatus>('start_fork_continuation', { args: { sessionId, rows, cols } }),
  pollContinuation: (handle: string) => invoke<ContinuationStatus>('poll_continuation', { args: { handle } }),
  writeContinuation: (handle: string, data: number[]) => invoke<void>('write_continuation', { args: { handle, data } }),
  resizeContinuation: (handle: string, rows: number, cols: number, pixelWidth = 0, pixelHeight = 0) => invoke<void>('resize_continuation', { args: { handle, rows, cols, pixelWidth, pixelHeight } }),
  closeContinuation: (handle: string) => invoke<void>('close_continuation', { args: { handle } }),
};
