# FORGE Wiki

A documentation wiki built entirely in FORGE, showcasing all 14 language primitives in a single, working application. Browse docs, search with LLM-powered confidence gating, ask questions, and auto-generate verified reference documentation — all defined in ~580 lines of FORGE.

## Prerequisites

- **FORGE** built from source: `cargo build` in the repo root
- **Anthropic API key** with access to Claude Sonnet

## Quick start

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
forge serve examples/wiki/server.forge -s examples/wiki/shared.forge --watch
```

Open [http://127.0.0.1:3000/home](http://127.0.0.1:3000/home).

The `--watch` flag enables hot-reload on file changes. Content from `content/` is seeded automatically on startup.

## Configuration

All configuration lives in [`forge.config.toml`](forge.config.toml):

```toml
[llm]
default = "wiki-provider"

[providers.wiki-provider]
type = "anthropic"
model = "claude-sonnet-4-20250514"
api_key = "${ANTHROPIC_API_KEY}"
timeout_secs = 60

[server]
host = "127.0.0.1"
port = 3000

[server.static]
root = "examples/wiki/static"
prefix = "/static"
```

| Key | Purpose |
|-----|---------|
| `providers.*.type` | LLM provider (`anthropic`, `openai`, etc.) |
| `providers.*.model` | Model ID for LLM calls |
| `providers.*.api_key` | API key — use `${ENV_VAR}` syntax to read from environment |
| `providers.*.timeout_secs` | Per-call timeout for LLM requests |
| `server.host` / `server.port` | Bind address and port |
| `server.static.root` | Directory for static files (CSS, JS, images) |
| `server.static.prefix` | URL prefix for static file routes |

## Content management

Wiki pages are markdown files in `content/`:

```
content/
  home.md                   Landing page
  getting-started.md        Installation and first program
  principles.md             The 9 first principles
  roadmap.md                Development roadmap
  reference/
    task.md                 task primitive reference
    agent.md                agent primitive reference
    flow.md                 flow primitive reference
    pool.md                 pool primitive reference
    system.md               system primitive reference
    event.md                event primitive reference
    states.md               states primitive reference
    when.md                 when primitive reference
    pure.md                 pure primitive reference
    boundary.md             boundary primitive reference
    requires.md             requires primitive reference
    warden.md               warden primitive reference
```

Pages are stored in ReDB via `data.store("page:{slug}", content)` and retrieved with `data.get("page:{slug}")`. The slug maps directly to the filename (without `.md`). To add a page, create a markdown file in `content/` and add its slug to the sidebar in the `docs_sidebar` pure function in `server.forge`.

## Endpoints

| Route | Method | Returns | Description |
|-------|--------|---------|-------------|
| `/home` | GET | Html | Landing page with hero section and feature cards |
| `/docs?slug=<slug>` | GET | Html | Render a documentation page by slug |
| `/search_page?q=<query>` | GET | Html | LLM-powered search with confidence gating |
| `/ask_form` | GET | Html | Question form for the Q&A agent |
| `/ask_page?question=<q>` | GET | Html | LLM answer with confidence badge |
| `/api_search?q=<query>` | GET | Text | JSON API for programmatic search |
| `/admin_generate_docs` | GET | Html | Trigger auto-reference doc generation flow |
| `/admin_fact_check?slug=<slug>` | GET | Html | Run fact-check pool on a specific page |

## Admin features

### Auto-reference generation

Visit `/admin_generate_docs` to trigger the `generate_docs` flow. This runs a 5-stage pipeline:

1. **scan_files** — loads all page content
2. **extract_tasks / extract_agents / extract_flows** — three LLM extractions run in parallel
3. **generate_reference** — combines extractions into a markdown reference doc
4. **fact_check** — verifies claims through a 3-worker majority-vote pool
5. **publish** — stores results and emits update events

The generated reference appears at `/docs?slug=auto-reference` and the fact-check report at `/docs?slug=fact-check-report`.

### Per-page fact checking

Visit `/admin_fact_check?slug=<slug>` to verify any page. Each claim is extracted, sent to 3 independent checkers, and classified as PASS / NEEDS_REVIEW / FAIL based on consensus.

## Project structure

```
examples/wiki/
  server.forge              Main application (~580 lines)
  shared.forge              Types, events, and state machine (shared boundary)
  client.forge              Client-side stub (future WASM target)
  forge.config.toml         Provider, server, and static file config
  ARCHITECTURE.md           System diagrams and feature map
  README.md                 This file
  content/                  Markdown documentation pages
    home.md
    getting-started.md
    principles.md
    roadmap.md
    reference/              Per-primitive reference docs (12 files)
  static/
    css/style.css           Custom styles (DaisyUI theme, code blocks, animations)
    js/app.js               Client interactivity (theme toggle, form interceptors)
    js/forge-highlight.js   Prism.js language definition for FORGE syntax
  .forge-data/
    server.redb             ReDB database (page content, agent memory, events)
```

## Deployment

### Production considerations

**Tailwind CSS** — The wiki uses Tailwind via CDN for development convenience. For production, replace with a built CSS file:

```bash
# Install standalone Tailwind CLI
curl -sLO https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-macos-arm64
chmod +x tailwindcss-macos-arm64

# Build production CSS
./tailwindcss-macos-arm64 -i static/css/style.css -o static/css/tailwind.out.css --minify
```

Then update `wiki_head` in `server.forge` to reference the local CSS instead of the CDN script tag.

**Environment variables:**

```bash
export ANTHROPIC_API_KEY="sk-ant-..."   # Required: LLM provider key
```

### Running with `forge serve`

```bash
forge serve examples/wiki/server.forge \
  -s examples/wiki/shared.forge \
  --watch                                # optional: hot-reload on changes
```

### Reverse proxy (nginx)

```nginx
server {
    listen 443 ssl;
    server_name wiki.example.com;

    ssl_certificate     /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

### Reverse proxy (Caddy)

```
wiki.example.com {
    reverse_proxy 127.0.0.1:3000
}
```

Caddy handles SSL automatically via Let's Encrypt.

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for system diagrams, data flow, supervision tree, and a complete map of every FORGE primitive used in this wiki.
