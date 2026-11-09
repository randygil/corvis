// Records the schemas the original OpenAPI compiler plugin derives from class-validator decorators
// (classValidatorShim, on by default) into class_validator_shim.json.
// Usage: npm install && node generate.mjs
import 'reflect-metadata';
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import ts from 'typescript';
import { Body, Controller, Module, Post } from '@nestjs/common';
import { NestFactory } from '@nestjs/core';
import { DocumentBuilder, SwaggerModule } from '@nestjs/swagger';

const require = createRequire(import.meta.url);
const plugin = require('@nestjs/swagger/plugin');
const here = dirname(fileURLToPath(import.meta.url));
const out = join(here, 'out');
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
// The compiled DTOs use require(): keep them out of this ES module package.
writeFileSync(join(out, 'package.json'), '{ "type": "commonjs" }');

const options = {
  target: ts.ScriptTarget.ES2022,
  module: ts.ModuleKind.CommonJS,
  moduleResolution: ts.ModuleResolutionKind.Node10,
  experimentalDecorators: true,
  emitDecoratorMetadata: true,
  strict: true,
  skipLibCheck: true,
  outDir: out,
  rootDir: here,
};
const program = ts.createProgram([join(here, 'cases.dto.ts')], options);
const result = program.emit(undefined, undefined, undefined, false, {
  before: [plugin.before({ classValidatorShim: true, introspectComments: false, dtoFileNameSuffix: ['.dto.ts'] }, program)],
});
const diagnostics = ts.getPreEmitDiagnostics(program).concat(result.diagnostics);
if (diagnostics.length > 0) {
  for (const d of diagnostics) console.error(ts.flattenDiagnosticMessageText(d.messageText, '\n'));
  process.exit(1);
}

const dtos = await import(pathToFileURL(join(out, 'cases.dto.js')).href);
const names = Object.keys(dtos).filter((n) => n.endsWith('Dto'));

class C {}
names.forEach((n, i) => {
  const method = `m${i}`;
  C.prototype[method] = Object.defineProperty(function () {}, 'name', { value: method });
  const d = Object.getOwnPropertyDescriptor(C.prototype, method);
  Reflect.defineMetadata('design:paramtypes', [dtos[n]], C.prototype, method);
  Body()(C.prototype, method, 0);
  Post(method)(C.prototype, method, d);
});
Controller('x')(C);
class M {}
Module({ controllers: [C] })(M);

const app = await NestFactory.create(M, { logger: false });
const doc = SwaggerModule.createDocument(app, new DocumentBuilder().setTitle('t').setVersion('1').build());
await app.close();

const schemas = {};
for (const n of names) schemas[n] = doc.components.schemas[n];
writeFileSync(
  join(here, 'class_validator_shim.json'),
  JSON.stringify({ versions: { swagger: '11.4.6', classValidator: '0.14.4' }, schemas }, null, 2) + '\n',
);
rmSync(out, { recursive: true, force: true });
console.log(`wrote ${names.length} schemas`);
