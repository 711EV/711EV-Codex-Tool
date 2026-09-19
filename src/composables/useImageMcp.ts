import { computed, onScopeDispose, ref, watch, type Ref } from "vue";
import { backend } from "../services/backend";
import { imageMcpService, isImageMcpReady } from "../services/imageMcpService";
import { useMessageStore } from "../stores/messages";

type Status = "idle" | "ready" | "repair";
interface ImagePrompt { profileId: string; context: string; mode: "restart" | "repair" }
const repairMessage = "生图功能受限，请点击修复生图";
const repairMessageKey = "image-mcp-update-failed";

export function useImageMcp(profileId: Ref<string | null>, providerId: Ref<string | null>, switching: Ref<boolean>) {
  const messages = useMessageStore();
  const statuses = ref<Record<string, Status>>({});
  const busy = ref(false);
  const checking = ref(false);
  const eligible = ref(false);
  const blockedByProvider = ref(false);
  const context = computed(() => JSON.stringify([profileId.value, providerId.value]));
  const prompt = ref<ImagePrompt | null>(null);
  const status = computed(() => eligible.value ? statuses.value[profileId.value ?? ""] ?? "idle" : "idle");
  const label = computed(() => busy.value
    ? (prompt.value?.mode === "repair" ? "正在修复" : "正在准备")
    : status.value === "repair" ? "修复生图" : status.value === "ready" ? "生图正常" : "使用生图");
  let refreshId = 0;
  let disposed = false;

  function failure(target: string, error: unknown) {
    statuses.value[target] = "repair";
    if (!disposed && target === profileId.value) {
      const detail = error instanceof Error ? error.message : String(error);
      messages.showMessage(`${repairMessage}：${detail}`, "danger", { duration: 0, key: repairMessageKey });
    }
  }

  async function refresh() {
    const request = ++refreshId;
    const target = profileId.value;
    eligible.value = false;
    blockedByProvider.value = false;
    messages.removeMessage(repairMessageKey);
    if (!target || busy.value || switching.value) { checking.value = false; return; }
    checking.value = true;
    try {
      const result = await imageMcpService.refreshIfInstalled(target);
      if (disposed || request !== refreshId || target !== profileId.value) return;
      eligible.value = Boolean(result?.eligible);
      blockedByProvider.value = result?.eligible === false;
      if (!eligible.value) prompt.value = null;
      statuses.value[target] = isImageMcpReady(result) ? "ready" : "idle";
      if (result?.eligible && (result.error || (result.configRegistered && !result.agentsConfigured))) {
        failure(target, result.error ?? "生图配置或工具文件需要修复");
      }
    } catch (error) {
      if (!disposed && request === refreshId && target === profileId.value) failure(target, error);
    } finally {
      if (request === refreshId) checking.value = false;
    }
  }

  async function prepare() {
    const target = profileId.value;
    if (!target || !eligible.value || busy.value || checking.value || switching.value) return;
    if (status.value === "repair") { prompt.value = { profileId: target, context: context.value, mode: "repair" }; return; }
    if (status.value === "ready") { messages.showMessage("生图功能正常", "success"); return; }
    busy.value = true;
    const request = ++refreshId;
    const startedContext = context.value;
    try {
      await imageMcpService.ensureInstalled(target);
      if (!disposed && request === refreshId && startedContext === context.value) {
        statuses.value[target] = "ready";
        messages.removeMessage(repairMessageKey);
        messages.showMessage("生图配置完成", "success");
        prompt.value = { profileId: target, context: startedContext, mode: "restart" };
      }
    } catch (error) {
      if (!disposed && request === refreshId && startedContext === context.value) {
        messages.showMessage(error instanceof Error ? error.message : String(error), "danger");
      }
    } finally {
      busy.value = false;
      if (!disposed && (request !== refreshId || startedContext !== context.value)) void refresh();
    }
  }

  async function confirm() {
    const action = prompt.value;
    if (!action || !eligible.value || busy.value || switching.value || action.context !== context.value) return;
    const request = ++refreshId;
    busy.value = true;
    try {
      if (action.mode === "repair") {
        await imageMcpService.repair(action.profileId);
      } else {
        await backend.restartCodexClient(action.profileId);
      }
      if (!disposed && request === refreshId && action.context === context.value) {
        if (action.mode === "repair") statuses.value[action.profileId] = "ready";
        messages.removeMessage(repairMessageKey);
        messages.showMessage(action.mode === "repair" ? "生图功能修复完毕" : "ChatGPT 已重启", "success");
      }
      prompt.value = null;
    } catch (error) {
      if (!disposed && request === refreshId && action.context === context.value) {
        if (action.mode === "repair") failure(action.profileId, error);
        else messages.showMessage(String(error), "danger");
      }
    } finally {
      busy.value = false;
      if (!disposed && (request !== refreshId || action.context !== context.value)) void refresh();
    }
  }

  function close() { if (!busy.value) prompt.value = null; }
  watch([context, switching], () => { prompt.value = null; void refresh(); }, { immediate: true });
  onScopeDispose(() => { disposed = true; ++refreshId; });
  return { busy, checking, eligible, blockedByProvider, status, label, prompt, prepare, confirm, close, refresh };
}
