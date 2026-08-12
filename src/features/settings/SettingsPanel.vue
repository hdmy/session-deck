<script setup lang="ts">
import { computed, onMounted, onUnmounted, shallowRef, useTemplateRef } from 'vue';
import { api } from '../../api';
import type { ClaudeSettings, IndexDiagnostics, ProviderDescriptor, ProviderId, ResumePreview, ScanReport, ScanSettings, SessionSummary, SourceRootActivationReport } from '../../types';
import IndexDiagnosticsPanel from './IndexDiagnosticsPanel.vue';
import ScanScheduleForm from './ScanScheduleForm.vue';
import SourceRootForm from './SourceRootForm.vue';
import { useI18n, type Locale } from '../../i18n';

const props = defineProps<{
  session: SessionSummary | null;
  scanning?: boolean;
  continuationActive?: boolean;
  providers?: readonly ProviderDescriptor[];
  activateSourceRoot?: (sourceRoot: string | null, replaceActiveIndexAcknowledged: boolean) => Promise<SourceRootActivationReport>;
}>();
const emit = defineEmits<{
  close: [];
  saved: [];
  rootActivated: [report: ScanReport];
  scanSettingsChanged: [settings: ScanSettings, scanScopeChanged: boolean];
}>();

const panel = useTemplateRef<HTMLElement>('panel');
const { locale, t, setLocale } = useI18n();
const localeDraft = shallowRef<Locale>(locale.value);
const settings = shallowRef<ClaudeSettings>({ executable_override: null, dangerously_skip_permissions: false });
const scanSettings = shallowRef<ScanSettings | null>(null);
const diagnostics = shallowRef<IndexDiagnostics | null>(null);
const executable = shallowRef('');
const skipPermissions = shallowRef(false);
const riskAcknowledged = shallowRef(false);
const sourceRoot = shallowRef('');
const replaceConfirmed = shallowRef(false);
const intervalSeconds = shallowRef(0);
const enabledProviderIds = shallowRef<ProviderId[]>(['claude']);
const providerLookbackDays = shallowRef<Partial<Record<ProviderId, number>>>({});
const preview = shallowRef<ResumePreview | null>(null);
const loading = shallowRef(true);
const claudeLoading = shallowRef(true);
const scanLoading = shallowRef(true);
const claudeReady = shallowRef(false);
const scanReady = shallowRef(false);
const claudeLoadError = shallowRef<string | null>(null);
const scanLoadError = shallowRef<string | null>(null);
const diagnosticsLoading = shallowRef(true);
const saving = shallowRef(false);
const scanSaving = shallowRef(false);
const rootSaving = shallowRef(false);
const error = shallowRef<string | null>(null);
const diagnosticsError = shallowRef<string | null>(null);
const notice = shallowRef<string | null>(null);
let restoreFocus: HTMLElement | null = null;
let diagnosticsGeneration = 0;
let settingsGeneration = 0;

const gateBusy = computed(() => Boolean(props.scanning || props.continuationActive));
const anySaving = computed(() => saving.value || scanSaving.value || rootSaving.value);
const gateReason = computed(() => props.continuationActive ? t('continuationSaveDisabled') : props.scanning ? t('scanningSaveDisabled') : null);
const claudeImportEnabled = computed(() => enabledProviderIds.value.includes('claude'));
const rootDisabledReason = computed(() => claudeImportEnabled.value ? gateReason.value : t('enableClaudeImport'));
const rootChanged = computed(() => sourceRoot.value.trim() !== (scanSettings.value?.source_root ?? '').trim());

function message(cause: unknown) { return cause instanceof Error ? cause.message : typeof cause === 'string' ? cause : t('localSettingsFailure'); }
function onLocaleChange(event: Event) {
  const value = (event.target as HTMLSelectElement).value;
  if (value === 'en' || value === 'zh') localeDraft.value = value as Locale;
}
function validInterval(value: number) { return value === 0 || (Number.isInteger(value) && value >= 60 && value <= 3600); }
function validateScanDraft(): boolean {
  if (validInterval(intervalSeconds.value)) return true;
  error.value = t('scanIntervalError');
  return false;
}
function reportNotice(report: ScanReport) {
  const outcome = report.outcome;
  if (report.committed === false || (outcome === 'partial' && !report.committed)) return t('rootScanNotCommitted');
  if (report.committed && report.partial) return t('rootScanPartial');
  return t('rootScanComplete', { outcome });
}

async function loadDiagnostics() {
  const token = ++diagnosticsGeneration;
  diagnosticsLoading.value = true;
  diagnosticsError.value = null;
  try {
    const next = await api.getIndexDiagnostics();
    if (token === diagnosticsGeneration) diagnostics.value = next;
  } catch (cause) {
    if (token === diagnosticsGeneration) diagnosticsError.value = message(cause);
  } finally {
    if (token === diagnosticsGeneration) diagnosticsLoading.value = false;
  }
}

async function loadScanSettings() {
  const token = ++settingsGeneration;
  scanLoading.value = true;
  scanLoadError.value = null;
  try {
    const scan = await api.getScanSettings();
    if (token !== settingsGeneration) return;
    scanSettings.value = scan;
    sourceRoot.value = scan.source_root ?? '';
    intervalSeconds.value = scan.scan_interval_seconds;
    enabledProviderIds.value = [...scan.enabled_provider_ids];
    providerLookbackDays.value = { ...scan.provider_lookback_days };
    scanReady.value = true;
  } catch (cause) {
    if (token === settingsGeneration) {
      scanReady.value = false;
      scanSettings.value = null;
      scanLoadError.value = message(cause);
    }
  } finally {
    if (token === settingsGeneration) scanLoading.value = false;
  }
}

async function load() {
  loading.value = true;
  claudeLoading.value = true;
  scanLoading.value = true;
  claudeReady.value = false;
  scanReady.value = false;
  claudeLoadError.value = null;
  scanLoadError.value = null;
  diagnosticsLoading.value = true;
  error.value = null;
  diagnosticsError.value = null;
  const claudePromise = api.getClaudeSettings().then((claude) => {
    settings.value = claude;
    executable.value = claude.executable_override ?? '';
    skipPermissions.value = claude.dangerously_skip_permissions;
    claudeReady.value = true;
  }).catch((cause) => { claudeLoadError.value = message(cause); }).finally(() => { claudeLoading.value = false; });
  const scanPromise = loadScanSettings();
  void loadDiagnostics();
  await Promise.all([claudePromise, scanPromise]);
  loading.value = false;
}

async function saveClaude() {
  if (!claudeReady.value || anySaving.value) return;
  if (skipPermissions.value && !settings.value.dangerously_skip_permissions && !riskAcknowledged.value) {
    error.value = t('riskError');
    return;
  }
  const shouldSaveScan = scanReady.value && scanSettings.value !== null;
  if (shouldSaveScan && !validateScanDraft()) return;
  saving.value = true; error.value = null; notice.value = null;
  try {
    settings.value = await api.updateClaudeSettings({ executableOverride: executable.value.trim() || null, dangerouslySkipPermissions: skipPermissions.value, riskAcknowledged: riskAcknowledged.value });
    if (shouldSaveScan) {
      const scanResult = await persistScanSettings();
      if (scanResult) emit('scanSettingsChanged', scanResult.settings, scanResult.scanScopeChanged);
    }
    setLocale(localeDraft.value);
    riskAcknowledged.value = false; notice.value = t('savedClaudeSettings'); emit('saved');
  } catch (cause) { error.value = message(cause); }
  finally { saving.value = false; }
}

async function persistScanSettings(): Promise<{ settings: ScanSettings; scanScopeChanged: boolean } | null> {
  const current = scanSettings.value;
  if (!current) return null;
  scanSaving.value = true;
  try {
    const previous = current.enabled_provider_ids;
    const previousLookbacks = current.provider_lookback_days;
    const next = await api.updateScanSettings({
      scan_interval_seconds: intervalSeconds.value,
      enabled_provider_ids: enabledProviderIds.value,
      provider_lookback_days: providerLookbackDays.value,
    });
    const providersChanged = previous.length !== next.enabled_provider_ids.length
      || previous.some((id) => !next.enabled_provider_ids.includes(id));
    const lookbacksChanged = Object.keys({ ...previousLookbacks, ...next.provider_lookback_days })
      .some((id) => previousLookbacks[id] !== next.provider_lookback_days[id]);
    scanSettings.value = next;
    intervalSeconds.value = next.scan_interval_seconds;
    enabledProviderIds.value = [...next.enabled_provider_ids];
    providerLookbackDays.value = { ...next.provider_lookback_days };
    return { settings: next, scanScopeChanged: providersChanged || lookbacksChanged };
  } finally {
    scanSaving.value = false;
  }
}

async function saveScanSettings() {
  const current = scanSettings.value;
  if (!scanReady.value || !current || anySaving.value) return;
  if (!validateScanDraft()) return;
  error.value = null; notice.value = null;
  try {
    const result = await persistScanSettings();
    if (result) {
      notice.value = result.scanScopeChanged ? t('scanSettingsSavedProviders') : t('scanSettingsSaved');
      emit('scanSettingsChanged', result.settings, result.scanScopeChanged);
    }
  } catch (cause) { error.value = message(cause); }
}

async function activateRoot() {
  if (gateBusy.value || !scanReady.value || anySaving.value) return;
  if (!rootChanged.value) { notice.value = t('rootUnchanged'); return; }
  if (!replaceConfirmed.value) { error.value = t('confirmReplaceError'); return; }
  rootSaving.value = true; error.value = null; notice.value = null;
  try {
    // App injects the lease-owning callback. The direct command is retained
    // only for mounting this panel in isolated component tests.
    const result = props.activateSourceRoot
      ? await props.activateSourceRoot(sourceRoot.value.trim() || null, true)
      : await api.activateClaudeSourceRoot(sourceRoot.value.trim() || null, true);
    await loadScanSettings();
    replaceConfirmed.value = false;
    const report = result.scan;
    await loadDiagnostics();
    notice.value = reportNotice(report);
    emit('rootActivated', report);
  } catch (cause) { error.value = message(cause); }
  finally { rootSaving.value = false; }
}

async function preflight() {
  if (!props.session) return;
  preview.value = null; error.value = null;
  try { preview.value = await api.resumePreflight(props.session.id); } catch (cause) { error.value = message(cause); }
}
function close() { emit('close'); }
function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') { event.preventDefault(); close(); return; }
  if (event.key !== 'Tab' || !panel.value) return;
  const focusable = [...panel.value.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), [tabindex]:not([tabindex="-1"])')];
  if (!focusable.length) return;
  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  if (!panel.value.contains(document.activeElement)) { event.preventDefault(); (event.shiftKey ? last : first).focus(); }
  else if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
}

onMounted(() => {
  restoreFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  document.addEventListener('keydown', onKey);
  void load();
  queueMicrotask(() => panel.value?.querySelector<HTMLElement>('.settings-close')?.focus());
});
onUnmounted(() => { document.removeEventListener('keydown', onKey); restoreFocus?.focus(); });
</script>

<template>
  <div class="settings-backdrop" role="presentation" @click.self="close">
    <section ref="panel" class="settings-panel" role="dialog" aria-modal="true" aria-labelledby="settings-title">
      <header class="settings-head"><div><span class="eyebrow">{{ t('localOnly') }}</span><h2 id="settings-title">{{ t('claudeSettings') }}</h2></div><button class="settings-close" type="button" :aria-label="t('closeSettings')" @click="close">×</button></header>
      <form class="settings-form" @submit.prevent="saveClaude">
        <div class="settings-content">
        <label class="language-field"><span>{{ t('language') }}</span><select :value="localeDraft" :aria-label="t('language')" @change="onLocaleChange"><option value="en">{{ t('languageEnglish') }}</option><option value="zh">{{ t('languageChinese') }}</option></select><span class="hint">{{ t('languageHelp') }}</span></label>
        <p v-if="claudeLoading" class="field-help" role="status">{{ t('claudeSettingsLoading') }}</p>
        <p v-else-if="claudeLoadError" class="settings-error" role="alert">{{ t('claudeSettingsUnavailable', { value: claudeLoadError }) }}</p>
        <label>{{ t('executableOverride') }} <span class="hint">{{ t('optional') }}</span><input v-model="executable" :disabled="!claudeReady" :placeholder="t('automaticClaude')" autocomplete="off" /></label>
        <p class="field-help">{{ t('executableHelp') }}</p>
        <label class="check-row"><input v-model="skipPermissions" :disabled="!claudeReady" type="checkbox" /> <span><strong>{{ t('skipPermissionPrompts') }}</strong><small>{{ t('skipPermissionHelp') }}</small></span></label>
        <label v-if="skipPermissions && !settings.dangerously_skip_permissions" class="risk-row"><input v-model="riskAcknowledged" :disabled="!claudeReady" type="checkbox" /> {{ t('riskConfirmation') }}</label>
        <div class="settings-divider"></div>
        <p v-if="scanLoading" class="field-help" role="status">{{ t('scanSettingsLoading') }}</p>
        <p v-else-if="scanLoadError" class="settings-error" role="alert">{{ t('scanSettingsUnavailable', { value: scanLoadError }) }}</p>
        <SourceRootForm v-if="scanReady && scanSettings" v-model:source-root="sourceRoot" v-model:replace-confirmed="replaceConfirmed" :effective-root="scanSettings.effective_root ?? ''" :disabled="gateBusy || !scanReady || anySaving || !claudeImportEnabled" :disabled-reason="rootDisabledReason" :saving="rootSaving" @activate="activateRoot" />
        <ScanScheduleForm v-if="scanReady && scanSettings" v-model:interval-seconds="intervalSeconds" v-model:enabled-provider-ids="enabledProviderIds" v-model:provider-lookback-days="providerLookbackDays" :providers="providers ?? []" :disabled="gateBusy || !scanReady || anySaving" :disabled-reason="gateReason" :saving="scanSaving" @save="saveScanSettings" />
        <IndexDiagnosticsPanel :diagnostics="diagnostics" :loading="diagnosticsLoading" :error="diagnosticsError" />
        <div class="settings-divider"></div>
        <div class="preflight-block"><div class="preflight-title"><span>{{ t('continuationPreflight') }}</span><button type="button" class="secondary-button" :disabled="!session" @click="preflight">{{ session ? t('checkSelectedSession') : t('selectSessionFirst') }}</button></div><p v-if="!session" class="field-help">{{ t('selectClaudeHelp') }}</p><dl v-if="preview" class="preview-list"><dt>{{ t('resolvedExecutable') }}</dt><dd>{{ preview.resolved_executable }}</dd><dt>{{ t('version') }}</dt><dd>{{ preview.version || t('unavailable') }}</dd><dt>{{ t('historicalCwd') }}</dt><dd>{{ preview.cwd }}</dd><dt>{{ t('readOnlyCommandPreview') }}</dt><dd><code>{{ preview.command_preview }}</code></dd></dl></div>
        </div>
        <footer class="settings-footer"><div class="settings-feedback"><p v-if="error" class="settings-error" role="alert">{{ error }}</p><p v-if="notice" class="settings-notice" role="status">{{ notice }}</p></div><div class="settings-actions"><button type="button" class="secondary-button" @click="close">{{ t('cancel') }}</button><button class="primary-button" type="submit" :disabled="saving || !claudeReady || anySaving">{{ saving ? t('saving') : t('saveSettings') }}</button></div></footer>
      </form>
    </section>
  </div>
</template>

<style scoped>
.settings-backdrop { position: fixed; inset: 0; z-index: 30; display: grid; place-items: center; background: #05080bcc; }.settings-panel { display: flex; flex-direction: column; width: min(620px, calc(100vw - 40px)); max-height: calc(100vh - 40px); overflow: hidden; padding: 24px; border: 1px solid #35464d; border-radius: 10px; background: #171f25; box-shadow: 0 20px 60px #0009; }.settings-head, .settings-footer, .preflight-title { display: flex; align-items: center; justify-content: space-between; gap: 12px; }.settings-head h2 { margin: 5px 0 0; font-size: 20px; font-weight: 600; }.settings-close { border: 0; background: transparent; color: #9aabb0; font-size: 24px; cursor: pointer; }.settings-form { display: flex; flex: 1 1 auto; min-height: 0; flex-direction: column; margin-top: 22px; overflow: hidden; }.settings-content { display: grid; flex: 1 1 auto; min-height: 0; gap: 14px; overflow-y: auto; padding-right: 4px; }.settings-form label { display: grid; gap: 7px; color: #d7e0e2; font-size: 12px; }.hint { color: #798a91; font-size: 10px; }.settings-form input:not([type=checkbox]) { width: 100%; box-sizing: border-box; padding: 9px 10px; border: 1px solid #34434a; border-radius: 5px; background: #10171c; color: #e6eeed; font: inherit; }.settings-form input:focus { outline: 2px solid #75b9a466; border-color: #75b9a4; }.field-help, .check-row small { margin: -4px 0 0; color: #7f9198; font-size: 11px; line-height: 1.45; }.check-row { grid-template-columns: auto 1fr !important; align-items: start; }.check-row input, .risk-row input { accent-color: #75b9a4; }.check-row span { display: grid; gap: 3px; }.risk-row { padding: 9px; border-left: 2px solid #bf9660; background: #73572b1c; grid-template-columns: auto 1fr !important; line-height: 1.4; }.settings-divider { height: 1px; background: #2b383e; }.preflight-block { display: grid; gap: 10px; }.preflight-title { color: #b8c8ca; font-size: 12px; }.secondary-button, .primary-button { border: 1px solid #3c4d53; border-radius: 5px; padding: 7px 10px; color: #b9c8c8; background: transparent; cursor: pointer; font: inherit; font-size: 11px; }.primary-button { border-color: #75b9a4; background: #75b9a41c; color: #c9e8dd; }.secondary-button:hover:not(:disabled), .primary-button:hover:not(:disabled) { background: #2a3d40; }.secondary-button:disabled, .primary-button:disabled { opacity: .5; cursor: not-allowed; }.preview-list { display: grid; grid-template-columns: 145px 1fr; gap: 8px; margin: 0; padding: 11px; border: 1px solid #2c3b42; border-radius: 5px; background: #11181d; font-size: 11px; }.preview-list dt { color: #778a92; }.preview-list dd { margin: 0; color: #d1dedd; overflow-wrap: anywhere; }.preview-list code { color: #d4c28a; }.settings-error { margin: 0; color: #dc9292; font-size: 11px; }.settings-notice { margin: 0; color: #8bc7b0; font-size: 11px; }.settings-footer { flex: 0 0 auto; margin: 14px -24px -24px; padding: 14px 24px 24px; border-top: 1px solid #2b383e; background: #171f25; }.settings-feedback { min-width: 0; text-align: left; }.settings-actions { display: flex; flex: 0 0 auto; gap: 12px; margin-left: auto; }.settings-state { min-height: 120px; display: grid; place-items: center; color: #829198; }
.language-field select { width: 100%; box-sizing: border-box; padding: 9px 10px; border: 1px solid #34434a; border-radius: 5px; background: #10171c; color: #e6eeed; font: inherit; }
</style>
