// Records what `normalizeIp` of `@nestjs/throttler` returns for a corpus of addresses and prefixes. Run once with
// `npm install && node normalize-ip.mjs`; the output (`normalize-ip.json`) is committed and replayed by the unit
// tests of `src/modules/throttler.rs` through `normalize_ip`. It starts no server.
import { writeFileSync } from 'node:fs';
import { normalizeIp } from '@nestjs/throttler';

const corpus = [
  '1.2.3.4', '0.0.0.0', '255.255.255.255', '01.2.3.4', '1.2.3.4%x',
  '::1', '0:0:0:0:0:0:0:1', '::', '::2',
  '2001:db8::1', '2001:DB8:0:0:1:0:0:1', '2001:0db8:0001:0002:0003:0004:0005:0006',
  '2001:db8:1:2:ffff:ffff:ffff:ffff', 'fe80::1%eth0', 'fe80::1%',
  '::ffff:1.2.3.4', '::FFFF:102:304', '::ffff:0:0', '::1.2.3.4', '64:ff9b::1.2.3.4',
  '1:2:3:4:5:6:1.2.3.4', '1::2:3:4:5:6:7', '1:2:3:4:5:6:7::', '0:0:1::1', '1:0:0:1::1',
  '[::1]', '1.2.3.4:80', '::ffff:1.2.3.256', '1:2:3:4:5:6:7:8:9', 'unknown', '',
  // Where `net.isIP` and a strict RFC parser could disagree.
  '1.2.3.04', '01.2.3.4%x', '1.2.3.4.', '1.2.3.4%', '1:2:3:4:5:6:7::', '::1:2:3:4:5:6:7', '1::2:3:4:5:6:7:8',
  '0001::', '00001::', '12345::', '::ffff:1.2.3.4.5', '1:2:3:4:5:6::1.2.3.4', '1:2:3:4:5::1.2.3.4', '::1.02.3.4',
  '::ffff:01.2.3.4', '::0.0.0.0', ':1::2', '1:::2', '1::2::3', ' ::1', '::1 ', '::1%eth0', '::ffff:1.2.3.4%eth0',
  '::%', '::FFFF:1.2.3.4', '2001:db8::', '1:0:0:0:1:0:0:0', '0:0:0:0:1:0:0:0', '1:2:0:0:0:0:3:4', '::ffff:ffff:ffff',
  '::fffe:1.2.3.4', '0:0:0:0:0:ffff:0102:0304', '2001:db8:1:2:3:4:5:6%eth0',
];
const prefixes = [0, 1, 15, 16, 17, 48, 56, 63, 64, 65, 96, 127, 128];

const out = [];
for (const ip of corpus) {
  for (const prefix of prefixes) out.push({ ip, prefix, out: normalizeIp(ip, prefix) });
}
writeFileSync(new URL('./normalize-ip.json', import.meta.url), JSON.stringify(out, null, 1) + '\n');
console.log(`wrote ${out.length} entries`);
