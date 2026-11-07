<script setup lang="ts">
import { computed } from 'vue'
import { useData } from 'vitepress'

const { frontmatter } = useData()

const labels: Record<string, string> = {
  parcial: 'Soporte parcial',
  'no-disponible': 'No disponible todavía',
}

const status = computed(() => String(frontmatter.value.status ?? 'documentado'))
const label = computed(() => labels[status.value])
</script>

<template>
  <p v-if="label" class="cv-status" :data-status="status">
    <span class="cv-status__dot" aria-hidden="true"></span>{{ label }}
  </p>
</template>
