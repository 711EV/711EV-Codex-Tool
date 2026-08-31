<script setup lang="ts">
import { useId } from "vue";
import { X } from "lucide-vue-next";

withDefaults(defineProps<{
  title: string;
  modalClass?: string;
  backdropClass?: string;
  closeLabel?: string;
  closeDisabled?: boolean;
}>(), {
  modalClass: "",
  backdropClass: "",
  closeLabel: "关闭",
  closeDisabled: false,
});

const emit = defineEmits<{
  close: [];
}>();
const headingId = `modal-heading-${useId()}`;
</script>

<template>
  <div class="modal-backdrop" :class="backdropClass">
    <section class="modal modal-dialog" :class="modalClass" role="dialog" aria-modal="true" :aria-labelledby="headingId">
      <header class="modal-heading">
        <h2 :id="headingId">{{ title }}</h2>
        <button type="button" class="icon-button" :disabled="closeDisabled" :aria-label="closeLabel" @click="emit('close')">
          <X :size="18" />
        </button>
      </header>
      <slot />
      <slot name="actions" />
    </section>
  </div>
</template>
