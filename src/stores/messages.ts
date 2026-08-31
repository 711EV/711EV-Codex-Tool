import { computed, ref } from "vue";
import { defineStore } from "pinia";

export type MessageTone = "neutral" | "info" | "success" | "warning" | "danger";

export interface AppMessage {
  id: string | number;
  content: string;
  tone: MessageTone;
  persistent: boolean;
}

const MAX_MESSAGE_COUNT = 3;
const MESSAGE_DURATION_MS = 8000;

export const useMessageStore = defineStore("messages", () => {
  const messages = ref<AppMessage[]>([]);
  const timers = new Map<string | number, ReturnType<typeof setTimeout>>();
  const deadlines = new Map<string | number, number>();
  const remaining = new Map<string | number, number>();
  let nextId = 0;

  function clearTimer(id: string | number) {
    const timer = timers.get(id);
    if (timer) clearTimeout(timer);
    timers.delete(id);
  }

  function removeMessage(id: string | number) {
    clearTimer(id);
    deadlines.delete(id);
    remaining.delete(id);
    messages.value = messages.value.filter((message) => message.id !== id);
  }

  function schedule(id: string | number, duration: number) {
    clearTimer(id);
    if (duration <= 0) return;
    deadlines.set(id, Date.now() + duration);
    remaining.delete(id);
    timers.set(id, setTimeout(() => removeMessage(id), duration));
  }

  function showMessage(
    content: string,
    tone: MessageTone = "neutral",
    options: { duration?: number; key?: string | number | null } = {},
  ) {
    const duration = options.duration ?? MESSAGE_DURATION_MS;
    const id = options.key ?? ++nextId;
    // Replacing a keyed message must cancel its previous countdown, including
    // when the replacement is persistent (duration: 0).
    clearTimer(id);
    deadlines.delete(id);
    remaining.delete(id);
    const index = messages.value.findIndex((message) => message.id === id);
    const message: AppMessage = {
      id,
      content,
      tone,
      persistent: duration === 0,
    };
    const next = index < 0
      ? [...messages.value, message]
      : messages.value.map((current, currentIndex) => currentIndex === index ? message : current);
    while (next.length > MAX_MESSAGE_COUNT) {
      let removable = next.findIndex((current) => !current.persistent);
      if (removable < 0) removable = 0;
      const [removed] = next.splice(removable, 1);
      clearTimer(removed.id);
    }
    messages.value = next;
    if (duration > 0 && messages.value.some((current) => current.id === id)) schedule(id, duration);
    return id;
  }

  function pauseMessage(id: string | number) {
    if (!timers.has(id)) return;
    const deadline = deadlines.get(id) ?? Date.now();
    clearTimer(id);
    deadlines.delete(id);
    remaining.set(id, Math.max(0, deadline - Date.now()));
  }

  function resumeMessage(id: string | number) {
    const duration = remaining.get(id);
    if (duration === undefined) return;
    remaining.delete(id);
    if (duration <= 0) removeMessage(id);
    else schedule(id, duration);
  }

  function dispose() {
    timers.forEach((timer) => clearTimeout(timer));
    timers.clear();
    deadlines.clear();
    remaining.clear();
    messages.value = [];
  }

  return {
    messages,
    messageCount: computed(() => messages.value.length),
    showMessage,
    removeMessage,
    pauseMessage,
    resumeMessage,
    dispose,
  };
});
