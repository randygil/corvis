import 'reflect-metadata';
import { INestApplication, ValidationPipe } from '@nestjs/common';
import { NestFactory } from '@nestjs/core';
import { AppModule } from './app.module';

// Variante de plataforma: `node dist/main.js` (Express, por defecto)
// o `node dist/main.js --fastify` (o PLATFORM=fastify).
const useFastify = process.argv.includes('--fastify') || process.env.PLATFORM === 'fastify';

async function bootstrap() {
  // Solo errores y advertencias: Nest no registra logs por request.
  const logger = ['error', 'warn'] as const;
  let app: INestApplication;
  if (useFastify) {
    // Import diferido para que la variante Express no cargue Fastify.
    const { FastifyAdapter } = await import('@nestjs/platform-fastify');
    app = await NestFactory.create(AppModule, new FastifyAdapter({ logger: false }), { logger: [...logger] });
  } else {
    app = await NestFactory.create(AppModule, { logger: [...logger] });
  }
  app.useGlobalPipes(new ValidationPipe());

  const port = Number(process.env.PORT ?? 3000);
  // IMPORTANTE: escuchar solo en loopback (nunca 0.0.0.0) para no disparar el firewall de Windows.
  await app.listen(port, '127.0.0.1');
}

bootstrap();
