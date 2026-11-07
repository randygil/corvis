<script setup lang="ts">
import { computed } from 'vue'
import { useData } from 'vitepress'

const { frontmatter, lang } = useData()

const labels: Record<string, Record<string, string>> = {
  en: {
    partial: 'Partial support',
    unavailable: 'Not available yet',
  },
  es: {
    partial: 'Soporte parcial',
    unavailable: 'No disponible todavía',
  },
}

const status = computed(() => String(frontmatter.value.status ?? 'documented'))
const label = computed(() => (labels[lang.value.startsWith('es') ? 'es' : 'en'] ?? labels.en)[status.value])
</script>

<template>
  <p v-if="label" class="cv-status" :data-status="status">
    <span class="cv-status__dot" aria-hidden="true"></span>{{ label }}
  </p>
</template>
