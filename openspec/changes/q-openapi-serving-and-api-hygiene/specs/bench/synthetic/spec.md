# Spec Delta

## ADDED Requirements

### Requirement: The benchmark measures the OpenAPI module
`bench/run.sh` SHALL offer the scenarios `hello-docs` (the `hello` route with the Swagger UI mounted), `docs-json` and
`docs-yaml` (the documents of a generated 300-operation application), run only against the Corvis application with
`CORVIS_BENCH_OPENAPI=1`, and report them as `n/a` for the Node application. Without that variable the Corvis
application MUST be the one the other scenarios measure.

#### Scenario: OpenAPI scenarios
- **WHEN** `SCENARIOS="hello hello-docs docs-json docs-yaml" bash bench/run.sh` runs
- **THEN** the results contain the four Corvis figures and `n/a` for the three OpenAPI scenarios of the Node app

#### Scenario: Default scenarios unchanged
- **WHEN** `bash bench/run.sh` runs with the default scenarios
- **THEN** the Corvis application starts without the OpenAPI module, as before
