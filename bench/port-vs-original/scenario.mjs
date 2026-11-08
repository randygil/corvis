// apidiff scenario that prepares the data of the port-versus-original benchmark and checks, before anything is
// measured, that both servers answer every measured request the same way.
//
// It logs in as the administrator, creates today's exchange rate and PRODUCTS products with fixed SKUs, then sends
// each measured request once (apidiff compares status, headers and body). The values the load generator needs (the
// token and the ids of each side) are written to the file named by BENCH_STATE_FILE, one entry per side:
//
//   { products: 100, a: { token, productId, previewBody }, b: { ... } }
//
// BENCH_EXPECT_PRODUCTS (default PRODUCTS) is the product count the listing must return; the orchestrator's check of
// its own abort path changes it. BENCH_SCENARIOS (comma list) selects which measured requests are sent.
import fs from 'node:fs';

export const PRODUCTS = 100;
const API = '/api/v1';
const TODAY = new Date().toISOString().slice(0, 10);
const TAX_CODES = ['G', 'G', 'R', 'GA', 'E'];
const QTY = ['2', '1', '3', '1.5', '0.375'];

export default async function (t) {
  const stateFile = t.env.BENCH_STATE_FILE;
  if (!stateFile) throw new Error('BENCH_STATE_FILE is not set');
  const expected = Number(t.env.BENCH_EXPECT_PRODUCTS ?? PRODUCTS);
  const wanted = (t.env.BENCH_SCENARIOS ?? 'health,products-list,product-get,invoice-preview').split(',');

  await t.get('/health', { expect: 200 });

  const login = await t.post(`${API}/auth/login`, {
    body: { email: t.env.BENCH_ADMIN_EMAIL, password: t.env.BENCH_ADMIN_PASSWORD },
    expect: 200,
  });
  const token = login.get('accessToken');
  const auth = { authorization: t.fmt`Bearer ${token}` };

  const ids = [];
  await t.group('data', async () => {
    await t.post(`${API}/bcv-rates`, { headers: auth, body: { valueDate: TODAY, rate: '40.2545' }, expect: 201 });
    for (let i = 1; i <= PRODUCTS; i++) {
      const n = String(i).padStart(3, '0');
      const r = await t.post(`${API}/products`, {
        headers: auth,
        body: {
          sku: `BENCH-${n}`,
          name: `Benchmark product ${n}`,
          description: `Fixed product ${n} of the port-versus-original benchmark`,
          priceUsd: (1 + (i % 17) * 1.35).toFixed(2),
          alicuota: TAX_CODES[i % TAX_CODES.length],
          unit: 'unidad',
        },
        expect: 201,
        name: `POST /products BENCH-${n}`,
      });
      ids.push(r.get('id'));
    }
  });

  await t.group('measured requests', async () => {
    if (wanted.includes('health')) await t.get('/health', { expect: 200 });

    if (wanted.includes('products-list')) {
      const list = await t.get(`${API}/products`, { headers: auth, expect: 200, name: 'GET /products' });
      const items = Array.isArray(list.body) ? list.body : (list.body?.items ?? list.body?.data ?? []);
      await t.check('product listing has the expected count', () =>
        items.length === expected ? null : `expected ${expected} products, the original returned ${items.length}`);
    }

    if (wanted.includes('product-get')) {
      await t.get(t.fmt`${API}/products/${ids[0]}`, { headers: auth, expect: 200, name: 'GET /products/:id' });
    }

    const previewBody = (side) => ({
      items: QTY.map((qty, k) => ({ productId: ids[k][side], qty })),
      payments: [{ method: 'EFECTIVO_USD', amount: '20' }],
    });
    if (wanted.includes('invoice-preview')) {
      await t.post(`${API}/invoices/preview`, {
        headers: auth,
        body: t.sided(previewBody('a'), previewBody('b')),
        expect: 200,
        name: 'POST /invoices/preview',
      });
    }

    if (wanted.includes('login')) {
      await t.post(`${API}/auth/login`, {
        body: { email: t.env.BENCH_ADMIN_EMAIL, password: t.env.BENCH_ADMIN_PASSWORD },
        expect: 200,
        name: 'POST /auth/login',
      });
    }

    const side = (s) => ({ token: token[s], productId: ids[0][s], previewBody: previewBody(s) });
    fs.writeFileSync(
      stateFile,
      JSON.stringify({ products: PRODUCTS, scenarios: wanted, a: side('a'), b: side('b') }, null, 2),
    );
  });
}
