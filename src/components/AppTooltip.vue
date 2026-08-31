<script setup lang="ts">
import { useId } from "vue";

const props = withDefaults(defineProps<{
  content: string;
  placement?: "top" | "top-start" | "top-end";
}>(), { placement: "top" });

const tooltipId = `app-tooltip-${useId()}`;
let pointerFocused = false;

function handlePointerDown() { pointerFocused = true; }
function handlePointerEnter(event: PointerEvent) {
  const active = document.activeElement;
  if (active?.matches?.(".app-tooltip-trigger") && active !== event.currentTarget) {
    (active as HTMLElement).blur();
  }
}
function handlePointerLeave(event: PointerEvent) {
  if (pointerFocused && document.activeElement === event.currentTarget) {
    (event.currentTarget as HTMLElement).blur();
  }
  pointerFocused = false;
}
</script>

<template>
  <div
    class="app-tooltip-trigger"
    :class="`app-tooltip-trigger--${props.placement}`"
    tabindex="0"
    :aria-describedby="tooltipId"
    @pointerdown="handlePointerDown"
    @pointerenter="handlePointerEnter"
    @pointerleave="handlePointerLeave"
  >
    <slot />
    <div :id="tooltipId" class="app-tooltip-popup" role="tooltip">
      <div class="app-tooltip-inner">{{ props.content }}</div>
      <span class="app-tooltip-arrow" aria-hidden="true" />
    </div>
  </div>
</template>
