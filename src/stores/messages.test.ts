import { createPinia, setActivePinia } from "pinia";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useMessageStore } from "./messages";

describe("desk-style message queue", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("limits messages to three and evicts non-persistent entries first", () => {
    setActivePinia(createPinia());
    const store = useMessageStore();
    store.showMessage("persistent", "warning", { duration: 0, key: "persistent" });
    store.showMessage("one");
    store.showMessage("two");
    store.showMessage("three");
    expect(store.messages.map((message) => message.content)).toEqual(["persistent", "two", "three"]);
  });

  it("replaces keyed messages and pauses/resumes their countdown", () => {
    vi.useFakeTimers();
    setActivePinia(createPinia());
    const store = useMessageStore();
    store.showMessage("first", "danger", { key: "same", duration: 1000 });
    vi.advanceTimersByTime(400);
    store.pauseMessage("same");
    store.showMessage("updated", "success", { key: "same", duration: 1000 });
    expect(store.messages).toHaveLength(1);
    expect(store.messages[0]?.content).toBe("updated");
    vi.advanceTimersByTime(999);
    expect(store.messages).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(store.messages).toHaveLength(0);
  });

  it("automatically removes ordinary messages after eight seconds", () => {
    vi.useFakeTimers();
    setActivePinia(createPinia());
    const store = useMessageStore();
    store.showMessage("hello");
    vi.advanceTimersByTime(7999);
    expect(store.messages).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(store.messages).toHaveLength(0);
  });
});
