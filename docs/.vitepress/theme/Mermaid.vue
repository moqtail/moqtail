<!--
  Renders a ```mermaid fence (turned into <Mermaid> in config.ts) on the client, and again when the colour scheme changes.
  mermaid is imported on first use, so pages without a diagram never load it.
-->
<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { useData } from 'vitepress'

const props = defineProps<{ code: string }>()
const { isDark } = useData()
const svg = ref('')
let renders = 0

async function render() {
  const { default: mermaid } = await import('mermaid')
  await document.fonts.ready
  mermaid.initialize({
    startOnLoad: false,
    theme: isDark.value ? 'dark' : 'default',
    fontFamily: getComputedStyle(document.documentElement).getPropertyValue('--vp-font-family-base'),
  })
  const result = await mermaid.render(`mermaid-${crypto.randomUUID()}-${renders++}`, decodeURIComponent(props.code))
  svg.value = result.svg
}

onMounted(render)
watch(isDark, render)
</script>

<template>
  <div class="mermaid" v-html="svg"></div>
</template>

<style scoped>
.mermaid {
  display: flex;
  justify-content: center;
  margin: 16px 0;
}

.mermaid :deep(p) {
  margin: 0;
  line-height: 1.5;
}
</style>
