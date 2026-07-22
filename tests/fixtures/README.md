# Integration Test Fixtures

Each TOML file is a complete `dot` configuration or a complete configuration template. Fixtures are grouped by the integration-test domain that owns them.

File names describe the expected result at that layer:

- `valid-*.toml`: accepted or successfully executed by the tested layer.
- `invalid-*.toml`: rejected or reported as failed by the tested layer.
- `*-template.toml`: requires explicit runtime token replacement before parsing.
- `*.expected.txt`: complete expected text, when a test has one stable textual result.

Template tokens are intentionally conspicuous, such as `__OS__`, `__PROGRAM__`, and `__SOURCE__`. Token replacement belongs to the integration test; the shared fixture loader only resolves and reads files.

Small TOML inline tables passed as individual CLI arguments and partial dynamically generated values remain in Rust because they are not complete `dot` manifests.
