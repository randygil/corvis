#!/usr/bin/env node
// Genera bench/results.md a partir de los JSON crudos de oha (bench/raw/*.json)
// y del archivo de metadatos que escribe run.sh (bench/raw/meta.json).
//
// Uso: node bench/report.js <dir_raw> <salida.md>

'use strict';
const fs = require('fs');
const path = require('path');
const os = require('os');

const rawDir = process.argv[2] || path.join(__dirname, 'raw');
const outFile = process.argv[3] || path.join(__dirname, 'results.md');

const meta = JSON.parse(fs.readFileSync(path.join(rawDir, 'meta.json'), 'utf8'));

const fmtInt = (n) => Math.round(n).toLocaleString('en-US');
const fmtMs = (s) => (s == null ? 'n/d' : (s * 1000).toFixed(2));
const fmtMb = (kb) => (kb == null || kb === '' ? 'n/d' : (Number(kb) / 1024).toFixed(1));

const scenarioLabel = {
  hello: 'GET / (texto)',
  json: 'GET /json',
  param: 'GET /users/:id (ParseIntPipe)',
  post: 'POST /users (ValidationPipe)',
};

const rows = [];
for (const sc of meta.scenarios) {
  for (const fw of meta.frameworks) {
    const file = path.join(rawDir, `${fw.id}-${sc}.json`);
    if (!fs.existsSync(file)) continue;
    const r = JSON.parse(fs.readFileSync(file, 'utf8'));
    const status = r.statusCodeDistribution || {};
    const errors = r.errorDistribution || {};
    const total = Object.values(status).reduce((a, b) => a + b, 0);
    const non2xx = Object.entries(status)
      .filter(([code]) => !code.startsWith('2'))
      .reduce((a, [, n]) => a + n, 0);
    const errCount = Object.values(errors).reduce((a, b) => a + b, 0);
    const rps = r.summary.requestsPerSec;
    const p = r.latencyPercentiles || {};
    const rss = (meta.rss[fw.id] || {})[sc] || {};
    rows.push({
      sc,
      fw,
      rps,
      total,
      non2xx,
      errCount,
      errors,
      p50: p.p50,
      p99: p.p99,
      rssLoad: rss.peak_kb,
    });
  }
}

let md = '';
md += '# Benchmark de throughput: Node.js vs Corvis\n\n';
md += `Generado: ${meta.date}\n\n`;
md += '## Entorno\n\n';
md += `- SO: ${meta.os}\n`;
md += `- CPU: ${os.cpus()[0]?.model?.trim() || 'n/d'} (${os.cpus().length} hilos lógicos)\n`;
md += `- Node: ${meta.node} · rustc: ${meta.rustc}\n`;
md += `- Generador de carga: ${meta.loadgen}\n`;
md += `- Parámetros: ${meta.conns} conexiones, ${meta.duration}s por escenario, warmup de ${meta.warmup}s, HTTP/1.1 keep-alive, 127.0.0.1\n`;
md += '- Cliente y servidor corren en la misma máquina (compiten por CPU).\n';
md += '- Corvis: build `--release`, runtime Tokio multi-hilo (un worker por núcleo salvo que se indique `TOKIO_WORKER_THREADS`).\n';
md += '- Node.js: `node dist/main.js` (compilado con tsc), un solo proceso (sin cluster).\n\n';

md += '## Resultados\n\n';
md += '| Escenario | Framework | req/s | req/min | p50 (ms) | p99 (ms) | Errores / non-2xx | RSS bajo carga (MB) |\n';
md += '|---|---|---:|---:|---:|---:|---:|---:|\n';
for (const r of rows) {
  md += `| ${scenarioLabel[r.sc] || r.sc} | ${r.fw.label} | ${fmtInt(r.rps)} | ${fmtInt(r.rps * 60)} | ${fmtMs(r.p50)} | ${fmtMs(r.p99)} | ${r.errCount} / ${r.non2xx} | ${fmtMb(r.rssLoad)} |\n`;
}
md += '\n';

// Relación Corvis vs Node.js (Express) por escenario.
const base = meta.frameworks.find((f) => f.id === 'node-express');
const mainCorvis = meta.frameworks.find((f) => f.id === 'corvis');
if (base && mainCorvis) {
  md += '## Relación de throughput (respecto a Node.js + Express)\n\n';
  md += '| Escenario | ' + meta.frameworks.map((f) => f.label).join(' | ') + ' |\n';
  md += '|---|' + meta.frameworks.map(() => '---:').join('|') + '|\n';
  for (const sc of meta.scenarios) {
    const b = rows.find((r) => r.sc === sc && r.fw.id === base.id);
    if (!b) continue;
    const cells = meta.frameworks.map((f) => {
      const r = rows.find((x) => x.sc === sc && x.fw.id === f.id);
      return r ? `${(r.rps / b.rps).toFixed(2)}x` : 'n/d';
    });
    md += `| ${scenarioLabel[sc] || sc} | ${cells.join(' | ')} |\n`;
  }
  md += '\n';
}

md += '## Memoria RSS en reposo (tras arrancar, antes de la carga)\n\n';
md += '| Framework | RSS en reposo (MB) | RSS máximo observado bajo carga (MB) |\n|---|---:|---:|\n';
for (const fw of meta.frameworks) {
  const rss = meta.rss[fw.id] || {};
  const peaks = Object.values(rss)
    .map((x) => Number(x && x.peak_kb))
    .filter((n) => !Number.isNaN(n) && n > 0);
  const peak = peaks.length ? Math.max(...peaks) : null;
  md += `| ${fw.label} | ${fmtMb(rss.idle_kb)} | ${fmtMb(peak)} |\n`;
}
md += '\n';
md += '_RSS = working set de Windows (`tasklist`), muestreado varias veces durante cada medición; es aproximado._\n\n';

const bad = rows.filter((r) => r.errCount > 0 || r.non2xx > 0);
md += '## Sanidad\n\n';
if (bad.length === 0) {
  md += '- Todas las mediciones terminaron con 0 errores de conexión y 0 respuestas non-2xx.\n';
} else {
  for (const r of bad) {
    md += `- ATENCIÓN: ${r.fw.label} / ${r.sc}: ${r.errCount} errores, ${r.non2xx} non-2xx. Detalle: \`${JSON.stringify(r.errors)}\`\n`;
  }
}
md += `- Antes de medir, run.sh comprueba con curl que cada endpoint devuelve exactamente el cuerpo y el código esperados en cada framework.\n`;
if (meta.notes) md += `- ${meta.notes}\n`;

md += '\n## Cómo interpretar\n\n';
md += '- **Corvis (todos los núcleos)** frente a Node.js es la comparación "tal cual sale de la caja": Tokio usa todos los núcleos y Node ejecuta el JS en un solo hilo. ';
md += 'En esta máquina, con cliente y servidor en el mismo equipo, el techo de oha + loopback de Windows ronda los ~100-110k req/s (un servidor hyper sin framework se queda en esa misma cifra), ';
md += 'así que esa fila mide sobre todo el techo del entorno, no el límite de Corvis.\n';
md += '- **Corvis (1 worker)** (`TOKIO_WORKER_THREADS=1`) es la comparación por núcleo, más justa frente a un proceso Node sin cluster.\n';
md += '- req/min = req/s × 60 (extrapolado de la ventana medida).\n';
md += '\n## Cómo reejecutar\n\n```bash\n# Desde la raíz del workspace, en Git Bash\nbash bench/run.sh\n\n# Parámetros opcionales (variables de entorno)\nCONNS=100 DURATION=15 WARMUP=5 OHA_THREADS=8 FRAMEWORKS="corvis corvis-1t node-express node-fastify" bash bench/run.sh\n```\n';

fs.writeFileSync(outFile, md);
console.log(`Escrito ${outFile}`);
