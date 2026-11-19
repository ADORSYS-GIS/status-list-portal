# Contributing

## Updating snapshots

When an intentional API change makes its snapshot test fail, run
`INSTA_UPDATE=always cargo test --lib openapi_snapshot`, review the snapshot
diff, then run `bun run test -u` to regenerate the frontend API types from it.
Commit both generated updates together after running `cargo test --all-targets --locked`
and `bun run test`. Other snapshot assertions in the integration tests are inline;
with `cargo-insta` installed, run `cargo insta review` to review and accept intentional
changes into their test source.

## Adding database migrations

Install Diesel CLI with PostgreSQL support at the version pinned in CI. Then create
the migration:

```sh
diesel migration generate <name>
```

Implement the forward and rollback SQL. Diesel CLI reads `DATABASE_URL`, while the
server reads `APP_DATABASE__URL`. Use a separate scratch database so tables from
other branches cannot affect the generated schema. With the local Docker database:

```sh
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/portal_schema
diesel database reset --no-default-migration
```

The reset drops and recreates the database named by `DATABASE_URL`, then applies
every migration. The flag prevents Diesel from adding its default setup migration
to the repository. Check that the URL points to the scratch database before running
it. Diesel CLI updates the checked-in schema automatically. Review that change and
run the tests. CI compares the schema against a fresh migrated database.
