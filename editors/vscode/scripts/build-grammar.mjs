// Generates syntaxes/corvis.injection.tmLanguage.json (run: node scripts/build-grammar.mjs).
// The attribute lists mirror crates/corvis-macros, crates/corvis-swagger-macros and corvis-orm-macros.
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const words = (s) => s.trim().split(/\s+/).join('|');

const DI = words('module controller injectable routes handlers websocket_gateway websocket_server global inject init optional');
const HTTP = words('get post put delete patch options head all search sse');
const ENHANCERS = words('use_guards use_interceptors use_pipes use_filters set_metadata apply_decorators http_code header redirect version');
const PARAMS = words('body param query headers ip req request ctx res response uploaded_file uploaded_files message_body connected_socket');
const EVENTS = words('on_event cron interval timeout subscribe_message');
const VALIDATION = words(`
  is_string is_int is_number is_boolean is_array is_object is_defined is_optional is_not_empty is_empty
  is_email is_url is_uuid is_alpha is_alphanumeric is_number_string is_numeric_string is_date_string
  is_iso8601 is_hex_color is_jwt is_mongo_id is_ip is_lowercase is_uppercase is_positive is_negative
  min_length max_length length min max matches contains not_contains is_in is_not_in equals
  not_equals array_min_size array_max_size array_not_empty array_unique is_enum validate_nested
  validate_with validate_if`);
const ORM = words('id unique relation updated_at map db db_type model orm_enum');

const attr = (names, nameScope, metaSuffix, body = '#attr-body') => ({
  name: `meta.attribute.rust meta.decorator.corvis.${metaSuffix}`,
  begin: `(#)\\s*(\\[)\\s*(?:(corvis)\\s*(::)\\s*)?(${names})\\b`,
  beginCaptures: {
    1: { name: 'punctuation.definition.attribute.rust punctuation.definition.decorator.corvis' },
    2: { name: 'punctuation.brackets.attribute.rust' },
    3: { name: 'entity.name.namespace.rust' },
    4: { name: 'keyword.operator.namespace.rust' },
    5: { name: nameScope },
  },
  end: '\\]',
  endCaptures: { 0: { name: 'punctuation.brackets.attribute.rust' } },
  patterns: [{ include: body }],
});

const nested = (open, close, scope, body) => ({
  begin: open,
  beginCaptures: { 0: { name: scope } },
  end: close,
  endCaptures: { 0: { name: scope } },
  patterns: [{ include: body }],
});

const bodyPatterns = (strings) => [
  { include: 'source.rust#block-comments' },
  { include: 'source.rust#comments' },
  { include: strings },
  {
    comment: 'key = value (imports = [...], path = "...", summary = "...")',
    match: '\\b(r#)?([A-Za-z_]\\w*)\\s*(=)(?![=>])',
    captures: {
      2: { name: 'variable.other.property.key.corvis' },
      3: { name: 'keyword.operator.assignment.rust' },
    },
  },
  {
    comment: 'bare `global` in #[module(global, ...)]',
    match: '\\b(global|passthrough)\\b(?=\\s*[,)])',
    name: 'variable.other.property.key.corvis',
  },
  { include: '#zod' },
  { include: 'source.rust#macros' },
  { include: 'source.rust#lifetimes' },
  { include: 'source.rust#constants' },
  { include: 'source.rust#keywords' },
  { include: 'source.rust#gtypes' },
  { include: 'source.rust#types' },
  { include: 'source.rust#namespaces' },
  { include: 'source.rust#lvariables' },
  { include: 'source.rust#variables' },
];

const grammar = {
  $schema: 'https://raw.githubusercontent.com/martinring/tmlanguage/master/tmlanguage.json',
  name: 'Corvis (Rust injection)',
  scopeName: 'corvis.injection',
  injectionSelector: 'L:source.rust -comment -string',
  patterns: [{ include: '#corvis-main' }, { include: '#attributes' }, { include: '#zod-dto' }, { include: '#zod' }],
  repository: {
    'corvis-main': {
      name: 'meta.attribute.rust meta.decorator.corvis.di',
      match: '(#)\\s*(\\[)\\s*(corvis)\\s*(::)\\s*(main|test)\\s*(\\])',
      captures: {
        1: { name: 'punctuation.definition.attribute.rust punctuation.definition.decorator.corvis' },
        2: { name: 'punctuation.brackets.attribute.rust' },
        3: { name: 'entity.name.namespace.rust' },
        4: { name: 'keyword.operator.namespace.rust' },
        5: { name: 'keyword.control.decorator.di.corvis' },
        6: { name: 'punctuation.brackets.attribute.rust' },
      },
    },
    attributes: {
      patterns: [
        attr(HTTP, 'support.type.decorator.http.corvis', 'http', '#route-body'),
        attr(DI, 'keyword.control.decorator.di.corvis', 'di'),
        attr(ENHANCERS, 'entity.name.function.decorator.enhancer.corvis', 'enhancer'),
        attr(PARAMS, 'variable.parameter.decorator.param.corvis', 'param'),
        attr(EVENTS, 'support.function.decorator.event.corvis', 'event'),
        attr('api_\\w+', 'entity.other.attribute-name.decorator.swagger.corvis', 'swagger'),
        attr(VALIDATION, 'storage.modifier.decorator.validation.corvis', 'validation'),
        attr(ORM, 'storage.modifier.decorator.orm.corvis', 'orm'),
      ],
    },
    'attr-body': {
      patterns: [
        nested('\\(', '\\)', 'punctuation.brackets.round.rust', '#attr-body'),
        nested('\\[', '\\]', 'punctuation.brackets.square.rust', '#attr-body'),
        nested('\\{', '\\}', 'punctuation.brackets.curly.rust', '#attr-body'),
        ...bodyPatterns('source.rust#strings'),
        { include: 'source.rust#punctuation' },
      ],
    },
    'route-body': {
      patterns: [
        nested('\\(', '\\)', 'punctuation.brackets.round.rust', '#route-body'),
        nested('\\[', '\\]', 'punctuation.brackets.square.rust', '#route-body'),
        ...bodyPatterns('#route-path'),
        { include: 'source.rust#punctuation' },
      ],
    },
    'route-path': {
      comment: 'route path literal: ":id" parameters and "*" wildcards',
      name: 'string.quoted.double.rust string.other.route-path.corvis',
      begin: '"',
      beginCaptures: { 0: { name: 'punctuation.definition.string.rust' } },
      end: '"',
      endCaptures: { 0: { name: 'punctuation.definition.string.rust' } },
      patterns: [
        { include: 'source.rust#escapes' },
        {
          match: '(:)([A-Za-z_]\\w*)(\\?)?',
          captures: {
            1: { name: 'punctuation.definition.parameter.route.corvis' },
            2: { name: 'variable.parameter.route.corvis' },
            3: { name: 'keyword.operator.optional.route.corvis' },
          },
        },
        { match: '\\*', name: 'keyword.operator.wildcard.route.corvis' },
      ],
    },
    'zod-dto': {
      comment: 'zod_dto!(Dto, schema)',
      match: '\\b(zod_dto)(!)',
      captures: {
        1: { name: 'entity.name.function.macro.rust support.function.macro.zod.corvis' },
        2: { name: 'entity.name.function.macro.rust' },
      },
    },
    zod: {
      comment: 'z::string(), z::coerce::number(), z::object! { ... }',
      match: '\\b(z)\\s*(::)\\s*(?:(coerce|iso|regexes|locales|time_precision)\\s*(::)\\s*)?([a-z_]\\w*)(!)?(?=\\s*[({\\[])',
      captures: {
        1: { name: 'entity.name.namespace.rust support.class.zod.corvis' },
        2: { name: 'keyword.operator.namespace.rust' },
        3: { name: 'entity.name.namespace.rust' },
        4: { name: 'keyword.operator.namespace.rust' },
        5: { name: 'support.function.zod.corvis' },
        6: { name: 'support.function.zod.corvis' },
      },
    },
  },
};

const out = fileURLToPath(new URL('../syntaxes/corvis.injection.tmLanguage.json', import.meta.url));
writeFileSync(out, JSON.stringify(grammar, null, 2) + '\n');
console.log('wrote', out);
