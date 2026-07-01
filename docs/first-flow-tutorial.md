# First Flow Tutorial

This is a short path for editing and validating your first Spark flow.

## Start Spark

Initialize a development home and run the server:

```bash
SPARK_HOME=~/.spark-dev cargo run -p spark-server --bin spark-server -- init
SPARK_HOME=~/.spark-dev cargo run -p spark-server --bin spark-server -- serve --host 127.0.0.1 --port 8010
```

In another shell, use the CLI against that server:

```bash
SPARK_API_BASE_URL=http://127.0.0.1:8010 cargo run -p spark-cli --bin spark -- flow list
```

## Edit A Flow

Packaged example flows live under [crates/spark-assets/assets/flows/examples](../crates/spark-assets/assets/flows/examples). Copy one into your Spark home before editing:

```bash
mkdir -p ~/.spark-dev/flows
cp crates/spark-assets/assets/flows/examples/simple-linear.dot ~/.spark-dev/flows/my-first-flow.dot
```

Validate direct flow edits:

```bash
cargo run -p spark-cli --bin spark -- flow validate --file ~/.spark-dev/flows/my-first-flow.dot --text
```

Launch a saved flow through the development server:

```bash
SPARK_API_BASE_URL=http://127.0.0.1:8010 cargo run -p spark-cli --bin spark -- run launch --flow my-first-flow.dot --project .
```

## References

- Full DOT reference: [crates/spark-assets/assets/guides/dot-authoring.md](../crates/spark-assets/assets/guides/dot-authoring.md)
- Operations guide: [crates/spark-assets/assets/guides/spark-operations.md](../crates/spark-assets/assets/guides/spark-operations.md)
- Change-request flow: [crates/spark-assets/assets/flows/software-development/implement-change-request.dot](../crates/spark-assets/assets/flows/software-development/implement-change-request.dot)
