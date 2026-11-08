import { defineConfig, type DefaultTheme } from 'vitepress'
import { nav, sidebar } from './sidebar.mts'

const repo = 'https://github.com/randygil/corvis'

const en: DefaultTheme.Config = {
  nav: nav('en'),
  sidebar: { '/': sidebar('en') },
  outline: { level: [2, 3], label: 'On this page' },
  docFooter: { prev: 'Previous page', next: 'Next page' },
  darkModeSwitchLabel: 'Appearance',
  lightModeSwitchTitle: 'Switch to light theme',
  darkModeSwitchTitle: 'Switch to dark theme',
  sidebarMenuLabel: 'Menu',
  returnToTopLabel: 'Return to top',
  langMenuLabel: 'Change language',
  notFound: {
    title: 'Page not found',
    quote: 'This path does not exist in the documentation.',
    linkText: 'Back to the introduction',
  },
  footer: { message: 'Released under the MIT License.' },
}

const es: DefaultTheme.Config = {
  nav: nav('es'),
  sidebar: { '/es/': sidebar('es') },
  outline: { level: [2, 3], label: 'En esta página' },
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
    link: '/es/',
  },
  footer: { message: 'Publicado bajo la licencia MIT.' },
}

export default defineConfig({
  title: 'Corvis',
  cleanUrls: true,
  lastUpdated: false,
  srcExclude: ['README.md', 'scripts/**'],
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
  locales: {
    root: {
      label: 'English',
      lang: 'en',
      description:
        'Modular backend framework for Rust: modules, dependency injection, controllers, pipes, guards and interceptors.',
      themeConfig: en,
    },
    es: {
      label: 'Español',
      lang: 'es',
      link: '/es/',
      description:
        'Framework backend modular para Rust: módulos, inyección de dependencias, controladores, pipes, guards e interceptors.',
      themeConfig: es,
    },
  },
  themeConfig: {
    logo: '/logo.svg',
    siteTitle: 'Corvis',
    socialLinks: [{ icon: 'github', link: repo }],
    search: {
      provider: 'local',
      options: {
        locales: {
          es: {
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
      },
    },
  },
})
