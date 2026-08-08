export interface SessionSummary {
  id: string;
  native_session_id?: string | null;
  provider_id: string;
  project_id: string;
  workspace_id?: string;
  project_path?: string | null;
  worktree_path?: string | null;
  title: string;
  source_title: string;
  hidden: boolean;
  pinned: boolean;
  last_used_at: number | null;
  started_at: number | null;
  ended_at: number | null;
  branch: string | null;
  first_prompt: string | null;
  last_prompt: string | null;
  cwd: string | null;
  models: string[];
  tool_count: number;
  source_mtime: number;
  partial: boolean;
}

export interface Project {
  id: string;
  workspace_id?: string;
  name: string;
  path: string;
  alias?: string | null;
  cwd_paths?: string[];
  worktree_paths?: string[];
  latest_activity: number | null;
  sessions: SessionSummary[];
}

export type TimelineEventKind =
  | 'user'
  | 'assistant'
  | 'thinking'
  | 'tool_use'
  | 'tool_result'
  | 'system'
  | 'unknown';

export interface TimelineEvent {
  id: number;
  session_id: string;
  kind: TimelineEventKind;
  role: string | null;
  content: string;
  timestamp: number | null;
  tool_name: string | null;
  collapsed: boolean;
  uuid?: string | null;
  parent_uuid?: string | null;
  logical_parent_uuid?: string | null;
  message_id?: string | null;
  parent_tool_use_id?: string | null;
  tool_use_id?: string | null;
  sequence?: number;
  is_sidechain?: boolean;
  is_meta?: boolean;
  turn_id?: number | null;
  final_response?: boolean;
  compact_boundary?: boolean;
  compact_preserved_ids?: string[];
}

export interface TurnActivity {
  event_id: number;
  kind: TimelineEventKind | string;
  role: string | null;
  content: string;
  timestamp: number | null;
  tool_name: string | null;
  tool_use_id: string | null;
  parent_tool_use_id: string | null;
  collapsed: boolean;
  final_response: boolean;
}

export interface ConversationTurn {
  id: number;
  session_id: string;
  user_prompt: string | null;
  activities: TurnActivity[];
  final_response: string | null;
  timestamp: number | null;
  completed: boolean;
}

/** A normalized, aggregate-only description of files touched by a turn. */
export interface FileChangeSummary {
  path: string;
  kind: string;
  additions: number;
  deletions: number;
  turn_id: number;
  event_id: number;
  tool_use_id: string | null;
}

/** Aggregate outcome counts for one provider tool name. */
export interface ToolStat {
  name: string;
  count: number;
  successes: number;
  failures: number;
  unknown: number;
  files_changed: number;
  additions: number;
  deletions: number;
}

/** Aggregate insights associated with a normalized conversation turn. */
export interface TurnInsight {
  turn_id: number;
  file_changes: FileChangeSummary[];
  tool_stats: ToolStat[];
}

export type BranchKind = 'active' | 'alternate' | 'compacted' | string;

export interface BranchSummary {
  id: string;
  session_id: string;
  label: string;
  kind: BranchKind;
  root_uuid: string | null;
  leaf_uuid: string | null;
  fork_point_uuid: string | null;
  is_active: boolean;
  event_count: number;
  turn_count: number;
  started_at: number | null;
  ended_at: number | null;
  compacted: boolean;
}

export interface Diagnostic {
  line: number;
  code: string;
}

export interface SessionDetail {
  summary: SessionSummary;
  timeline: TimelineEvent[];
  turns?: ConversationTurn[];
  diagnostics: Diagnostic[];
  branches?: BranchSummary[];
  active_branch_id?: string | null;
  selected_branch_id?: string | null;
  tool_stats?: ToolStat[] | null;
  turn_insights?: TurnInsight[] | null;
  relations?: SessionRelation[];
  cwd_history?: ObservedCwd[];
}

export interface ObservedCwd {
  cwd: string;
  first_sequence: number;
  last_sequence: number;
  first_timestamp: number | null;
  last_timestamp: number | null;
  resume: boolean;
}

export interface SessionRelation {
  provider_id: string;
  parent_session_id: string;
  child_session_id: string;
  relation_type: string;
  created_at: number;
  status: string;
  parent_present: boolean;
  child_present: boolean;
}

export interface SearchHit {
  session: SessionSummary;
  snippet: string;
  event_id: number;
}

export type ScanTrigger = 'manual' | 'scheduled' | 'post_continuation';
export type ScanOutcome = 'indexed' | 'committed' | 'partial' | 'failed' | 'skipped_lifecycle' | string;

export interface ScanReport {
  root: string;
  trigger: ScanTrigger | string;
  outcome: ScanOutcome;
  committed: boolean;
  sessions: number;
  diagnostics: number;
  partial: boolean;
  removed_sessions: number;
  new_files: number;
  changed_files: number;
  unchanged_files: number;
  removed_files: number;
  partial_sessions: number;
}

export interface ScanSettings {
  source_root: string | null;
  effective_root: string;
  scan_interval_seconds: number;
}

export interface ScanSettingsUpdate {
  scan_interval_seconds: number;
}

export interface ScanRun {
  id: number;
  trigger: ScanTrigger | string;
  outcome: ScanOutcome;
  started_at: number;
  committed: boolean;
}

export interface DiagnosticCount {
  code: string;
  count: number;
  last_occurred_at: number;
  last_run_id: number;
}

export interface IndexDiagnostics {
  effective_root: string;
  scan_interval_seconds: number;
  last_success_at: number | null;
  last_attempt_at: number | null;
  last_outcome: ScanOutcome | null;
  indexed_sessions: number;
  last_run: ScanRun | null;
  diagnostic_counts: DiagnosticCount[];
}

export interface SourceRootActivationReport {
  settings: ScanSettings;
  scan: ScanReport;
}

export interface AppStatus {
  root: string;
  indexed_sessions: number;
  last_scan_at: number | null;
  scan_in_progress: boolean;
}

export interface ClaudeSettings {
  executable_override: string | null;
  dangerously_skip_permissions: boolean;
}

export interface ClaudeSettingsUpdate {
  executableOverride?: string | null;
  dangerouslySkipPermissions?: boolean;
  riskAcknowledged?: boolean;
}

export interface ResumePreview {
  resolved_executable: string;
  version: string;
  cwd: string;
  args: string[];
  command_preview: string;
}

export interface ContinuationEvent {
  kind: 'output' | 'exited' | 'error' | string;
  data: number[] | null;
  status: string | null;
  message: string | null;
}

export interface LiveTranscriptEvent {
  id: string | number;
  kind: TimelineEventKind | string;
  role: string | null;
  content: string;
  timestamp: number | null;
  tool_name: string | null;
  collapsed: boolean;
}

export interface ContinuationStatus {
  handle: string;
  session_id: string;
  parent_session_id: string | null;
  status: string;
  events: ContinuationEvent[];
  live_events: LiveTranscriptEvent[];
  tail_partial: boolean;
  tail_diagnostics: number;
  tail_error: string | null;
  tail_caught_up: boolean;
}
