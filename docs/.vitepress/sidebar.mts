import type { DefaultTheme } from 'vitepress'

export type Locale = 'en' | 'es'

/** A label in both languages. */
type Label = { en: string; es: string }

type Page = { label: Label; link: string }
type Group = { label: Label; collapsed: boolean; pages: Page[] }
type Entry = Page | Group

const page = (en: string, es: string, link: string): Page => ({ label: { en, es }, link })
const group = (en: string, es: string, pages: [string, string, string][], collapsed = true): Group => ({
  label: { en, es },
  collapsed,
  pages: pages.map(([pen, pes, link]) => page(pen, pes, link)),
})

/**
 * Navigation tree, shared by both languages so they always list the same pages in the same order.
 * Links are written without the locale prefix; `sidebar('es')` adds `/es`.
 */
const tree: Entry[] = [
  page('Introduction', 'Introducción', '/'),
  group(
    'Overview',
    'Visión general',
    [
      ['First steps', 'Primeros pasos', '/first-steps'],
      ['Controllers', 'Controladores', '/controllers'],
      ['Providers', 'Providers', '/providers'],
      ['Modules', 'Módulos', '/modules'],
      ['Middleware', 'Middleware', '/middleware'],
      ['Exception filters', 'Filtros de excepción', '/exception-filters'],
      ['Pipes', 'Pipes', '/pipes'],
      ['Guards', 'Guards', '/guards'],
      ['Interceptors', 'Interceptors', '/interceptors'],
      ['Custom decorators', 'Decoradores personalizados', '/custom-decorators'],
    ],
    false,
  ),
  group('Fundamentals', 'Fundamentos', [
    ['Custom providers', 'Providers personalizados', '/fundamentals/custom-providers'],
    ['Async providers', 'Providers asíncronos', '/fundamentals/async-providers'],
    ['Dynamic modules', 'Módulos dinámicos', '/fundamentals/dynamic-modules'],
    ['Injection scopes', 'Scopes de inyección', '/fundamentals/injection-scopes'],
    ['Circular dependency', 'Dependencias circulares', '/fundamentals/circular-dependency'],
    ['Module reference', 'Referencia al módulo', '/fundamentals/module-ref'],
    ['Lazy-loading modules', 'Carga diferida de módulos', '/fundamentals/lazy-loading-modules'],
    ['Execution context', 'Contexto de ejecución', '/fundamentals/execution-context'],
    ['Lifecycle events', 'Eventos del ciclo de vida', '/fundamentals/lifecycle-events'],
    ['Discovery service', 'Servicio de descubrimiento', '/fundamentals/discovery-service'],
    ['Platform agnosticism', 'Independencia de la plataforma', '/fundamentals/platform-agnosticism'],
    ['Testing', 'Testing', '/fundamentals/testing'],
  ]),
  group('Application', 'Aplicación', [
    ['Configuration', 'Configuración', '/application/configuration'],
    ['Validation', 'Validación', '/application/validation'],
    ['Serialization', 'Serialización', '/application/serialization'],
    ['Logging', 'Logging', '/application/logger'],
    ['Events', 'Eventos', '/application/events'],
    ['Task scheduling', 'Tareas programadas', '/application/task-scheduling'],
    ['Queues', 'Colas', '/application/queues'],
    ['HTTP client', 'Cliente HTTP', '/application/http-client'],
    ['Internationalization', 'Internacionalización', '/application/i18n'],
    ['Mail', 'Correo', '/application/mail'],
    ['File storage', 'Almacenamiento de archivos', '/application/file-storage'],
  ]),
  group('Data', 'Datos', [
    ['Overview', 'Visión general', '/data/overview'],
    ['TypeORM', 'TypeORM', '/data/typeorm'],
    ['Drizzle', 'Drizzle', '/data/drizzle'],
    ['MongoDB', 'MongoDB', '/data/mongodb'],
    ['corvis-orm', 'corvis-orm', '/data/corvis-orm'],
    ['MikroORM', 'MikroORM', '/data/mikroorm'],
    ['Sequelize', 'Sequelize', '/data/sequelize'],
    ['Caching', 'Caché', '/data/caching'],
  ]),
  group('Security', 'Seguridad', [
    ['Authentication', 'Autenticación', '/security/authentication'],
    ['Authorization', 'Autorización', '/security/authorization'],
    ['Encryption and hashing', 'Cifrado y hashing', '/security/encryption-and-hashing'],
    ['Security headers', 'Cabeceras de seguridad', '/security/helmet'],
    ['CORS', 'CORS', '/security/cors'],
    ['CSRF protection', 'Protección CSRF', '/security/csrf'],
    ['Rate limiting', 'Límite de peticiones', '/security/rate-limiting'],
  ]),
  group('HTTP', 'HTTP', [
    ['Versioning', 'Versionado', '/http/versioning'],
    ['Cookies', 'Cookies', '/http/cookies'],
    ['Session', 'Sesión', '/http/session'],
    ['File upload and streaming', 'Subida de archivos y streaming', '/http/file-upload'],
    ['Compression', 'Compresión', '/http/compression'],
    ['Streaming responses', 'Respuestas en streaming', '/http/streaming-responses'],
    ['Server-Sent Events', 'Server-Sent Events', '/http/server-sent-events'],
    ['Model-View-Controller', 'Modelo-Vista-Controlador', '/http/mvc'],
    ['HTTP adapters', 'Adaptadores HTTP', '/http/performance'],
    ['Webhooks', 'Webhooks', '/http/webhooks'],
  ]),
  group('Observability', 'Observabilidad', [
    ['Overview', 'Visión general', '/observability/overview'],
    ['SDK', 'SDK', '/observability/sdk'],
    ['Manual instrumentation', 'Instrumentación manual', '/observability/manual-instrumentation'],
    ['Distributed tracing', 'Trazas distribuidas', '/observability/distributed-tracing'],
    ['Error monitoring', 'Monitoreo de errores', '/observability/error-monitoring'],
    ['Dashboard', 'Panel', '/observability/dashboard'],
    ['Production rules', 'Reglas de producción', '/observability/production-rules'],
    ['MCP server', 'Servidor MCP', '/observability/mcp-server'],
    ['OpenTelemetry', 'OpenTelemetry', '/observability/opentelemetry'],
    ['Choosing an APM', 'Elegir un APM', '/observability/choosing-an-apm'],
  ]),
  group('Reliability', 'Fiabilidad', [
    ['Resilience', 'Resiliencia', '/reliability/resilience'],
    ['Idempotency keys', 'Claves de idempotencia', '/reliability/idempotency'],
    ['Transactional outbox', 'Outbox transaccional', '/reliability/outbox'],
    ['Durable workflows', 'Flujos de trabajo durables', '/reliability/workflows'],
    ['Distributed locks', 'Bloqueos distribuidos', '/reliability/locks'],
    ['Health checks', 'Health checks', '/reliability/terminus'],
  ]),
  group('GraphQL', 'GraphQL', [
    ['Quick start', 'Inicio rápido', '/graphql/quick-start'],
    ['Resolvers', 'Resolvers', '/graphql/resolvers'],
    ['Mutations', 'Mutaciones', '/graphql/mutations'],
    ['Subscriptions', 'Suscripciones', '/graphql/subscriptions'],
    ['Scalars', 'Escalares', '/graphql/scalars'],
    ['Directives', 'Directivas', '/graphql/directives'],
    ['Interfaces', 'Interfaces', '/graphql/interfaces'],
    ['Unions and enums', 'Uniones y enums', '/graphql/unions-and-enums'],
    ['Field middleware', 'Middleware de campo', '/graphql/field-middleware'],
    ['Mapped types', 'Tipos mapeados', '/graphql/mapped-types'],
    ['Plugins', 'Plugins', '/graphql/plugins'],
    ['Complexity', 'Complejidad', '/graphql/complexity'],
    ['Extensions', 'Extensiones', '/graphql/extensions'],
    ['CLI plugin', 'Plugin de la CLI', '/graphql/cli-plugin'],
    ['Generating SDL', 'Generar SDL', '/graphql/generating-sdl'],
    ['Sharing models', 'Compartir modelos', '/graphql/sharing-models'],
    ['Other features', 'Otras características', '/graphql/other-features'],
    ['Federation', 'Federación', '/graphql/federation'],
  ]),
  group('WebSockets', 'WebSockets', [
    ['Gateways', 'Gateways', '/websockets/gateways'],
    ['Exception filters', 'Filtros de excepción', '/websockets/exception-filters'],
    ['Pipes', 'Pipes', '/websockets/pipes'],
    ['Guards', 'Guards', '/websockets/guards'],
    ['Interceptors', 'Interceptors', '/websockets/interceptors'],
    ['Adapters', 'Adaptadores', '/websockets/adapter'],
  ]),
  group('MCP', 'MCP', [
    ['Overview', 'Visión general', '/mcp/overview'],
    ['Primitives', 'Primitivas', '/mcp/primitives'],
    ['Transports', 'Transportes', '/mcp/transports'],
    ['Testing', 'Testing', '/mcp/testing'],
    ['Migration', 'Migración', '/mcp/migration'],
  ]),
  group('AI', 'IA', [
    ['Overview', 'Visión general', '/ai/overview'],
    ['Models', 'Modelos', '/ai/models'],
    ['Tools', 'Herramientas', '/ai/tools'],
    ['Agents', 'Agentes', '/ai/agents'],
    ['Embeddings', 'Embeddings', '/ai/embeddings'],
    ['Testing', 'Testing', '/ai/testing'],
  ]),
  group('Microservices', 'Microservicios', [
    ['Overview', 'Visión general', '/microservices/basics'],
    ['Redis', 'Redis', '/microservices/redis'],
    ['MQTT', 'MQTT', '/microservices/mqtt'],
    ['NATS', 'NATS', '/microservices/nats'],
    ['RabbitMQ', 'RabbitMQ', '/microservices/rabbitmq'],
    ['Kafka', 'Kafka', '/microservices/kafka'],
    ['gRPC', 'gRPC', '/microservices/grpc'],
    ['Custom transporters', 'Transportes personalizados', '/microservices/custom-transport'],
    ['Exception filters', 'Filtros de excepción', '/microservices/exception-filters'],
    ['Pipes', 'Pipes', '/microservices/pipes'],
    ['Pre-request hooks', 'Hooks previos a la petición', '/microservices/pre-request-hooks'],
    ['Guards', 'Guards', '/microservices/guards'],
    ['Interceptors', 'Interceptors', '/microservices/interceptors'],
  ]),
  page('Deployment', 'Despliegue', '/deployment'),
  page('Standalone applications', 'Aplicaciones standalone', '/standalone-applications'),
  group('CLI', 'CLI', [
    ['Overview', 'Visión general', '/cli/overview'],
    ['Workspaces', 'Workspaces', '/cli/monorepo'],
    ['Libraries', 'Librerías', '/cli/libraries'],
    ['Usage', 'Uso', '/cli/usages'],
    ['Scripts', 'Scripts', '/cli/scripts'],
  ]),
  group('OpenAPI', 'OpenAPI', [
    ['Introduction', 'Introducción', '/openapi/introduction'],
    ['Types and parameters', 'Tipos y parámetros', '/openapi/types-and-parameters'],
    ['Operations', 'Operaciones', '/openapi/operations'],
    ['Security', 'Seguridad', '/openapi/security'],
    ['Mapped types', 'Tipos mapeados', '/openapi/mapped-types'],
    ['Decorators', 'Decoradores', '/openapi/decorators'],
    ['CLI plugin', 'Plugin de la CLI', '/openapi/cli-plugin'],
    ['Other features', 'Otras características', '/openapi/other-features'],
  ]),
  group('Recipes', 'Recetas', [
    ['REPL', 'REPL', '/recipes/repl'],
    ['CRUD generator', 'Generador CRUD', '/recipes/crud-generator'],
    ['SWC (fast compiler)', 'SWC (compilador rápido)', '/recipes/swc'],
    ['Passport (authentication)', 'Passport (autenticación)', '/recipes/passport'],
    ['Hot reload', 'Recarga en caliente', '/recipes/hot-reload'],
    ['Router module', 'Módulo de rutas', '/recipes/router-module'],
    ['Migrating from Fastify', 'Migrar desde Fastify', '/recipes/fastify-migration'],
    ['CQRS', 'CQRS', '/recipes/cqrs'],
    ['Serve static', 'Archivos estáticos', '/recipes/serve-static'],
    ['Next.js', 'Next.js', '/recipes/nextjs'],
    ['Supabase', 'Supabase', '/recipes/supabase'],
    ['Commander', 'Commander', '/recipes/commander'],
    ['Async local storage', 'Async local storage', '/recipes/async-local-storage'],
    ['Socket activation', 'Activación por socket', '/recipes/socket-activation'],
    ['Migrating from Express', 'Migrar desde Express', '/recipes/express-migration'],
  ]),
  group('FAQ', 'Preguntas frecuentes', [
    ['Corvis vs Fastify, Express and Hono', 'Corvis frente a Fastify, Express y Hono', '/faq/corvis-vs-fastify-express-hono'],
    ['Serverless', 'Serverless', '/faq/serverless'],
    ['Cloudflare Workers', 'Cloudflare Workers', '/faq/cloudflare-workers'],
    ['HTTP adapter', 'Adaptador HTTP', '/faq/http-adapter'],
    ['Keep-alive connections', 'Conexiones keep-alive', '/faq/keep-alive-connections'],
    ['Global prefix', 'Prefijo global', '/faq/global-prefix'],
    ['Raw body', 'Cuerpo sin procesar', '/faq/raw-body'],
    ['Hybrid application', 'Aplicación híbrida', '/faq/hybrid-application'],
    ['HTTPS and multiple servers', 'HTTPS y varios servidores', '/faq/multiple-servers'],
    ['Request lifecycle', 'Ciclo de vida de la petición', '/faq/request-lifecycle'],
    ['Common errors', 'Errores comunes', '/faq/common-errors'],
    ['Examples', 'Ejemplos', '/faq/examples'],
  ]),
  group('Devtools', 'Devtools', [
    ['Overview', 'Visión general', '/devtools/overview'],
    ['CI/CD integration', 'Integración CI/CD', '/devtools/ci-cd-integration'],
  ]),
  page('Migration guide', 'Guía de migración', '/migration-guide'),
  page('API reference', 'Referencia de la API', '/api-reference'),
  page('Official courses', 'Cursos oficiales', '/courses'),
  group('Discover', 'Descubrir', [
    ['Who is using Corvis?', '¿Quién usa Corvis?', '/discover/companies'],
    ['Jobs board', 'Bolsa de empleo', '/discover/jobs'],
  ]),
  page('Support us', 'Apóyanos', '/support'),
]

/** Prefixes a link with the locale's base path. */
export const localize = (link: string, locale: Locale): string =>
  locale === 'es' ? (link === '/' ? '/es/' : `/es${link}`) : link

type Item = DefaultTheme.SidebarItem

const toItem = (e: Entry, locale: Locale): Item =>
  'pages' in e
    ? { text: e.label[locale], collapsed: e.collapsed, items: e.pages.map((p) => toItem(p, locale)) }
    : { text: e.label[locale], link: localize(e.link, locale) }

/** The sidebar of one locale. */
export function sidebar(locale: Locale): Item[] {
  return tree.map((e) => toItem(e, locale))
}

const navEntries: { label: Label; link: string; match: string }[] = [
  {
    label: { en: 'Guide', es: 'Guía' },
    link: '/first-steps',
    match: '(first-steps|controllers|providers|modules|middleware|exception-filters|pipes|guards|interceptors|custom-decorators)',
  },
  { label: { en: 'Fundamentals', es: 'Fundamentos' }, link: '/fundamentals/custom-providers', match: 'fundamentals/' },
  { label: { en: 'WebSockets', es: 'WebSockets' }, link: '/websockets/gateways', match: 'websockets/' },
  { label: { en: 'OpenAPI', es: 'OpenAPI' }, link: '/openapi/introduction', match: 'openapi/' },
  { label: { en: 'CLI', es: 'CLI' }, link: '/cli/overview', match: 'cli/' },
]

/** The top navigation bar of one locale. */
export function nav(locale: Locale): DefaultTheme.NavItem[] {
  const base = locale === 'es' ? '^/es/' : '^/'
  return navEntries.map((n) => ({
    text: n.label[locale],
    link: localize(n.link, locale),
    activeMatch: base + n.match,
  }))
}

/** Every page link of the tree without locale prefix, in reading order (used by the docs checks). */
export function allLinks(): string[] {
  return tree.flatMap((e) => ('pages' in e ? e.pages.map((p) => p.link) : [e.link]))
}
