# PXE Operations Console

This directory contains the static Next.js console used in two modes:

- `npm run build:demo` creates the read-only Vercel preview with simulated telemetry.
- `npm run build:embedded` creates the live same-origin console embedded into the Rust server.

## Local development

```bash
npm install
npm run dev
```

The development server uses demo mode unless `NEXT_PUBLIC_DASHBOARD_MODE=live` is set.

## Vercel

Create a Vercel project with this directory as its project root. The standard `npm run build` command produces the read-only demo because demo is the transport default. Branches and pull requests can therefore be reviewed without access to a PXE server or private network.

## Embedded build

Run the following before compiling or packaging the Rust application after a console change:

```bash
npm ci
npm run build:embedded
```

The generated `out/` directory is embedded by `rust-embed`. `macos/build_app.sh` performs these steps automatically.
