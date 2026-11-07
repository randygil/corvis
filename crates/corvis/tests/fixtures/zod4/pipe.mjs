// Generates `pipe.json`: responses of the real zod pipe packages (zod 4.4.3) for `tests/zod_pipe.rs`.
//
// Usage (from a folder with `zod@4.4.3`, `nestjs-zod@5.5.0` and `@nestjs/common` installed):
//   node <repo>/crates/corvis/tests/fixtures/zod4/pipe.mjs > <repo>/crates/corvis/tests/fixtures/zod4/pipe.json
import { createRequire } from "node:module";

const require = createRequire(process.cwd() + "/");
require("reflect-metadata");
const { z } = require("zod");
const { BadRequestException } = require("@nestjs/common");
const nz = require("nestjs-zod");

const CreateUser = z.strictObject({
  email: z.email(),
  password: z.string().min(1),
  role: z.enum(["admin", "user"]).default("user"),
});
class CreateUserDto extends nz.createZodDto(CreateUser) {}

const Query = z.object({
  page: z.coerce.number().int().min(1).default(1),
  includeInactive: z.stringbool().optional(),
});

const Conflict = z.intersection(z.string().transform(() => 1), z.string().transform(() => 2));

function run(f) {
  try {
    return { ok: f() };
  } catch (e) {
    return { status: e.getStatus?.(), body: e.getResponse?.(), name: e.constructor.name };
  }
}

const body = { type: "body", metatype: CreateUserDto, data: undefined };
const out = {
  // `new ZodValidationPipe(schema)` and the global pipe with a `createZodDto` class.
  pipeSchemaOk: run(() => new nz.ZodValidationPipe(CreateUser).transform({ email: "a@b.co", password: "x" }, body)),
  pipeSchemaFail: run(() => new nz.ZodValidationPipe(CreateUser).transform({ email: "nope", password: "", extra: 1 }, body)),
  pipeDtoFail: run(() => new nz.ZodValidationPipe().transform({}, body)),
  pipeDtoUndefined: run(() => new nz.ZodValidationPipe().transform(undefined, body)),
  pipeNoDto: run(() => new nz.ZodValidationPipe().transform({ a: 1 }, { type: "body", metatype: Object })),
  pipeQueryOk: run(() => new nz.ZodValidationPipe(Query).transform({ page: "3", includeInactive: "yes" }, { type: "query" })),
  pipeQueryFail: run(() => new nz.ZodValidationPipe(Query).transform({ page: "0", includeInactive: "maybe" }, { type: "query" })),
  // An error thrown while parsing (not a ZodError): the body has no `errors`.
  pipeThrown: run(() => new nz.ZodValidationPipe(Conflict).transform("x", { type: "body" })),
  strictNoDto: run(() => {
    const P = nz.createZodValidationPipe({ strictSchemaDeclaration: true });
    return new P().transform({ a: 1 }, { type: "body", metatype: Object });
  }),
  customException: run(() => {
    const P = nz.createZodValidationPipe({
      createValidationException: (error) => new BadRequestException({ custom: true, count: error.issues.length }),
    });
    return new P().transform({}, body);
  }),
  validateFail: run(() => nz.validate({ email: 1 }, CreateUserDto)),
  serializationException: run(() => {
    throw new nz.ZodSerializationException(new Error("x"));
  }),
};
process.stdout.write(JSON.stringify(out, null, 1) + "\n");
