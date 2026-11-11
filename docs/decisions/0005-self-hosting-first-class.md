# 0005: Self-Hosting Is First-Class

Status: accepted

Anyone must be able to run their own server with the same code and features as the main instance. Being able to self-host is how the project earns trust: people can check what the server does and run it themselves.

So the server stays one portable binary, configured through environment variables, with SQLite for small installs and Postgres for larger ones. Any hosted service it uses, such as object storage for attachments, sits behind a standard interface (S3-compatible) with a local default. No feature may depend on a single vendor's platform.

Considered instead: building the server natively on one edge platform (for example Workers and Durable Objects). That is cheaper to run at small sizes and needs no operations work, but only one vendor can host it.
