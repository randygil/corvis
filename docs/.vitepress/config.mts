import { defineConfig } from 'vitepress'
import { sidebar } from './sidebar.mts'

const repo = 'https://github.com/randygil/corvis'

export default defineConfig({
  lang: 'es',
  title: 'Corvis',
  description: 'Framework backend modular para Rust: módulos, inyección de dependencias, controladores, pipes, guards e interceptors.',
  cleanUrls: true,
  lastUpdated: false,
  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/logo.svg' }],
    ['link', { rel: 'preconnect', href: 'https://fonts.googleapis.com' }],
    ['link', { rel: 'preconnect', href: 'https://fonts.gstatic.com', crossorigin: '' }],
    [
      'link',
      {
        rel: 'stylesheet',
        href: 'https://fonts.googleapis.com/css2?family=Fraunces:opsz,wght@9..144,500;9..144,650&family=IBM+Plex+Sans:wght@400;500;600&family=JetBrains+Mono:wght@400;600&display=swap',
      },
    ],
  ],
  markdown: {
    theme: { light: 'github-light', dark: 'vitesse-dark' },
  },
  themeConfig: {
    logo: '/logo.svg',
    siteTitle: 'Corvis',
    nav: [
      { text: 'Guía', link: '/first-steps', activeMatch: '^/(first-steps|controllers|providers|modules|middleware|exception-filters|pipes|guards|interceptors|custom-decorators)' },
      { text: 'Fundamentos', link: '/fundamentals/custom-providers', activeMatch: '^/fundamentals/' },
      { text: 'WebSockets', link: '/websockets/gateways', activeMatch: '^/websockets/' },
      { text: 'OpenAPI', link: '/openapi/introduction', activeMatch: '^/openapi/' },
      { text: 'CLI', link: '/cli/overview', activeMatch: '^/cli/' },
    ],
    sidebar: { '/': sidebar },
    outline: { level: [2, 3], label: 'En esta página' },
    socialLinks: [{ icon: 'github', link: repo }],
    search: {
      provider: 'local',
      options: {
        translations: {
          button: { buttonText: 'Buscar', buttonAriaLabel: 'Buscar' },
          modal: {
            noResultsText: 'Sin resultados para',
            resetButtonTitle: 'Limpiar búsqueda',
            displayDetails: 'Mostrar detalles',
            backButtonTitle: 'Cerrar',
            footer: { selectText: 'seleccionar', navigateText: 'navegar', closeText: 'cerrar' },
          },
        },
      },
    },
    docFooter: { prev: 'Anterior', next: 'Siguiente' },
    darkModeSwitchLabel: 'Tema',
    lightModeSwitchTitle: 'Cambiar a tema claro',
    darkModeSwitchTitle: 'Cambiar a tema oscuro',
    sidebarMenuLabel: 'Menú',
    returnToTopLabel: 'Volver arriba',
    langMenuLabel: 'Idioma',
    notFound: {
      title: 'Página no encontrada',
      quote: 'Esta ruta no existe en la documentación.',
      linkText: 'Volver a la introducción',
    },
    footer: {
      message: 'Publicado bajo la licencia MIT.',
    },
  },
})
