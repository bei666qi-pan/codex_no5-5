import { applyStaticTranslations, loadLocale, saveLocale, t } from "./i18n.js";
import { statusView } from "./state.js";

const invoke = window.__TAURI__.core.invoke;
let current = null;
let daemonError = null;
let locale = loadLocale();

const $ = (id) => document.getElementById(id);

function setText(id, value) {
  $(id).textContent = value;
}

function render(status) {
  current = status;
  const view = statusView(status, locale);
  document.body.dataset.tone = view.meta.tone;
  $("status-card").dataset.status = view.key;
  setText("status-badge", view.meta.badge);
  setText("status-title", view.guidance.title);
  setText("status-detail", view.guidance.detail);
  $("status-dot").className = `dot ${view.meta.dot}`;
  setText("relay", view.relay);
  setText("upstream", view.upstream);
  setText("upstream-source", view.upstreamSource);
  setText("latency", view.latency);
  setText("remote", view.remote);
  setText("pause", view.pauseLabel);

  const heroAction = $("hero-action");
  heroAction.textContent = view.guidance.action_label || "";
  heroAction.classList.toggle("hidden", !view.showHeroAction);
  $("open-codex").classList.toggle("primary", !view.showHeroAction);
  $("open-codex").classList.toggle("secondary", view.showHeroAction);

  $("diagnostic").classList.toggle("hidden", !view.showNotice);
  if (view.showNotice) {
    setText("diagnostic-class", view.diagnosticTitle);
    setText("diagnostic-message", view.diagnosticMessage);
    const action = $("diagnostic-action");
    action.textContent = view.guidance.action_label || "";
    action.classList.toggle("hidden", !view.guidance.action_label);
  }
}

function renderDaemonUnavailable(error) {
  daemonError = error;
  document.body.dataset.tone = "danger";
  setText("status-badge", t(locale, "daemon_badge"));
  setText("status-title", t(locale, "daemon_title"));
  setText("status-detail", t(locale, "daemon_detail"));
  $("status-dot").className = "dot failed";
  $("onboarding").classList.remove("hidden");
  $("diagnostic").classList.remove("hidden");
  setText("diagnostic-class", t(locale, "daemon_not_running"));
  setText("diagnostic-message", String(error));
  $("diagnostic-action").classList.add("hidden");
}

async function load(method = "guard_status") {
  try {
    render(await invoke(method));
    daemonError = null;
    $("onboarding").classList.add("hidden");
  } catch (error) {
    renderDaemonUnavailable(error);
  }
}

async function showOutput(value) {
  $("output").classList.remove("hidden");
  $("output").textContent = typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

async function runGuidanceAction() {
  const action = current?.guidance?.action;
  try {
    if (action === "refresh") return load("refresh_guard");
    if (action === "resume_protection") return render(await invoke("set_paused", { paused: false }));
    if (action === "open_codex") return invoke("open_codex");
    if (action === "wait") {
      const path = await invoke("export_diagnostic");
      return showOutput(t(locale, "exported_diagnostic", { path }));
    }
  } catch (error) {
    return showOutput(String(error));
  }
}

async function runQuietAction(task) {
  try {
    await task();
  } catch (error) {
    await showOutput(String(error));
  }
}

$("refresh").addEventListener("click", () => load("refresh_guard"));
$("hero-action").addEventListener("click", runGuidanceAction);
$("diagnostic-action").addEventListener("click", runGuidanceAction);
$("open-codex").addEventListener("click", () => runQuietAction(() => invoke("open_codex")));
$("remote-start").addEventListener("click", () => runQuietAction(async () => showOutput(await invoke("remote_action", { action: "start" }))));
$("remote-pair").addEventListener("click", () => runQuietAction(async () => showOutput(await invoke("remote_action", { action: "pair" }))));
$("doctor").addEventListener("click", () => runQuietAction(async () => showOutput(await invoke("doctor"))));
$("export-diagnostic").addEventListener("click", () => runQuietAction(async () => {
  const path = await invoke("export_diagnostic");
  await showOutput(t(locale, "exported_diagnostic", { path }));
}));
$("pause").addEventListener("click", () => runQuietAction(async () => render(await invoke("set_paused", { paused: !current?.paused }))));
$("upstream-form").addEventListener("submit", (event) => runQuietAction(async () => {
  event.preventDefault();
  render(await invoke("set_upstream", { url: $("upstream-input").value }));
  await showOutput(t(locale, "upstream_enabled"));
}));
$("upstream-auto").addEventListener("click", () => runQuietAction(async () => {
  render(await invoke("set_upstream", { url: "auto" }));
  await showOutput(t(locale, "upstream_auto_enabled"));
}));
$("install").addEventListener("click", async () => {
  const button = $("install");
  button.disabled = true;
  button.textContent = t(locale, "checking");
  try {
    const probe = await invoke("onboarding_probe");
    $("step-codex").classList.toggle("done", probe.codex_found);
    $("step-vpn").classList.toggle("done", probe.healthy_count > 0);
    if (!probe.codex_found) throw new Error(t(locale, "codex_missing"));
    if (!probe.healthy_count) throw new Error(t(locale, "vpn_missing"));
    button.textContent = t(locale, "enabling");
    const service = await invoke("install_guard");
    $("step-service").classList.toggle("done", service.running);
    $("restart-tip").classList.remove("hidden");
    $("migrate-legacy").classList.toggle("hidden", !service.legacy_guard_detected);
    await load();
  } catch (error) {
    await showOutput(String(error));
  } finally {
    button.disabled = false;
    button.textContent = t(locale, "install");
  }
});
$("migrate-legacy").addEventListener("click", () => runQuietAction(async () => {
  await showOutput(await invoke("migrate_legacy_guard"));
  $("migrate-legacy").classList.add("hidden");
}));

$("locale").value = locale;
$("locale").addEventListener("change", (event) => {
  locale = event.target.value;
  saveLocale(locale);
  applyStaticTranslations(locale);
  if (current) render(current);
  else if (daemonError) renderDaemonUnavailable(daemonError);
});

applyStaticTranslations(locale);
load();
setInterval(() => load(), 10_000);
