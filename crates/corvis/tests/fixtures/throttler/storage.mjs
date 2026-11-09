// Records what `ThrottlerStorageService.increment` of `@nestjs/throttler` returns for scripted call sequences with a
// controlled clock. Run once with `npm install && node storage.mjs`; the output (`storage.json`) is committed and replayed
// by the unit tests of `src/modules/throttler.rs` through `ThrottlerStorage::record_hit`. It starts no server.
import { writeFileSync } from 'node:fs';
import { ThrottlerStorageService } from '@nestjs/throttler';

const BASE = 1_700_000_000_000;
let now = BASE;
Date.now = () => now;

// Each scenario is [ttl, limit, blockDuration, keys, times]: call i uses keys[i % keys.length] at `times[i]`
// (milliseconds after the start of the scenario).
const scenarios = {
  block_with_retries: [60000, 2, 60000, ['a'], [0, 1000, 2000, 30000, 61000, 62000, 63000, 64000, 125000]],
  sliding_window: [10000, 3, 10000, ['a'], [0, 9500, 9700, 10100, 10300, 10500, 19000, 20200, 20400]],
  block_longer_than_ttl: [60000, 2, 300000, ['a'], [0, 1000, 2000, 63000, 301000, 302000, 303000, 304000]],
  block_shorter_than_ttl: [60000, 2, 5000, ['a'], [0, 1000, 2000, 3000, 6000, 7000, 8000, 9000, 10000, 13000]],
  block_zero: [10000, 2, 0, ['a'], [0, 1000, 2000, 3000, 9000, 10000, 10500, 11000, 11500, 12000, 21500]],
  // Hits closer together than ttl / 64 (156 ms): Corvis groups them in buckets, so only the blocked state and the
  // times are compared, within ttl / 64.
  dense_hits: [10000, 5, 10000, ['a'], [0, 50, 100, 150, 200, 250, 300, 5000, 10100, 10400, 10500]],
  interleaved_keys: [10000, 2, 10000, ['a', 'b'], [0, 100, 200, 300, 400, 500, 600, 9000, 10250, 10350, 10450]],
};
const out = [];
for (const [name, [ttl, limit, blockDuration, keys, times]] of Object.entries(scenarios)) {
  const storage = new ThrottlerStorageService();
  const calls = [];
  for (const [i, t] of times.entries()) {
    now = BASE + t;
    const key = keys[i % keys.length];
    const result = await storage.increment(key, ttl, limit, blockDuration, 'default');
    calls.push({ t, key, ttl, limit, blockDuration, result });
  }
  storage.onApplicationShutdown();
  out.push({ name, calls });
}
// A different ttl, limit and block per key in one storage.
{
  const storage = new ThrottlerStorageService();
  const script = [
    [0, 'fast', 1000, 2, 1000],
    [100, 'slow', 5000, 3, 8000],
    [200, 'fast', 1000, 2, 1000],
    [300, 'slow', 5000, 3, 8000],
    [400, 'fast', 1000, 2, 1000],
    [500, 'slow', 5000, 3, 8000],
    [600, 'slow', 5000, 3, 8000],
    [1500, 'fast', 1000, 2, 1000],
    [1700, 'fast', 1000, 2, 1000],
    [8000, 'slow', 5000, 3, 8000],
    [8700, 'slow', 5000, 3, 8000],
  ];
  const calls = [];
  for (const [t, key, ttl, limit, blockDuration] of script) {
    now = BASE + t;
    const result = await storage.increment(key, ttl, limit, blockDuration, 'default');
    calls.push({ t, key, ttl, limit, blockDuration, result });
  }
  storage.onApplicationShutdown();
  out.push({ name: 'different_ttl_per_key', calls });
}
writeFileSync(new URL('./storage.json', import.meta.url), JSON.stringify(out, null, 1) + '\n');
